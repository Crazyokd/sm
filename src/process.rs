use crate::privacy::redact_text;
use anyhow::{Context, Result, bail};
use nix::{
    errno::Errno,
    fcntl::{FcntlArg, OFlag, fcntl},
    poll::{PollFd, PollFlags, poll},
    sys::signal::{Signal, killpg},
    unistd::Pid,
};
use std::{
    io::{self, Read, Write},
    os::fd::AsFd,
    os::unix::process::CommandExt,
    process::{Command, Output, Stdio},
    time::{Duration, Instant},
};

fn nonblocking(pipe: &impl AsFd) -> Result<()> {
    let flags = OFlag::from_bits_truncate(fcntl(pipe, FcntlArg::F_GETFL)?);
    fcntl(pipe, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK))?;
    Ok(())
}

fn read_pipe(pipe: &mut Option<impl Read>, bytes: &mut Vec<u8>) -> io::Result<()> {
    let Some(stream) = pipe else { return Ok(()) };
    let mut buffer = [0; 16384];
    match stream.read(&mut buffer) {
        Ok(0) => *pipe = None,
        Ok(count) => bytes.extend_from_slice(&buffer[..count]),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) => {}
        Err(error) => return Err(error),
    }
    Ok(())
}

pub fn run_command(
    mut command: Command,
    input: Option<&[u8]>,
    timeout: Duration,
) -> Result<Output> {
    let program = command.get_program().to_string_lossy().into_owned();
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = command
        .spawn()
        .with_context(|| format!("cannot start {program}"))?;
    let result = (|| -> Result<Output> {
        let mut stdout = child.stdout.take();
        let mut stderr = child.stderr.take();
        let mut stdin = child.stdin.take();
        for pipe in stdout
            .iter()
            .map(|p| p.as_fd())
            .chain(stderr.iter().map(|p| p.as_fd()))
            .chain(stdin.iter().map(|p| p.as_fd()))
        {
            nonblocking(&pipe)?;
        }
        let started = Instant::now();
        let mut status = None;
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let mut pending = input.unwrap_or_default();
        loop {
            if started.elapsed() >= timeout {
                bail!("{program} timed out");
            }
            read_pipe(&mut stdout, &mut out)?;
            read_pipe(&mut stderr, &mut err)?;
            if let Some(pipe) = &mut stdin {
                if !pending.is_empty() {
                    match pipe.write(pending) {
                        Ok(count) => pending = &pending[count..],
                        Err(error)
                            if matches!(
                                error.kind(),
                                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                            ) => {}
                        Err(error) => return Err(error.into()),
                    }
                }
                if pending.is_empty() {
                    stdin = None;
                }
            }
            if status.is_none() {
                status = child.try_wait()?;
            }
            if let Some(status) = status {
                if stdout.is_none() && stderr.is_none() && stdin.is_none() {
                    return Ok(Output {
                        status,
                        stdout: out,
                        stderr: err,
                    });
                }
            }
            let mut pipes = Vec::new();
            if let Some(pipe) = &stdout {
                pipes.push(PollFd::new(pipe.as_fd(), PollFlags::POLLIN));
            }
            if let Some(pipe) = &stderr {
                pipes.push(PollFd::new(pipe.as_fd(), PollFlags::POLLIN));
            }
            if let Some(pipe) = &stdin {
                pipes.push(PollFd::new(pipe.as_fd(), PollFlags::POLLOUT));
            }
            let remaining = timeout.saturating_sub(started.elapsed());
            let interval = remaining.as_millis().min(20) as u16;
            match poll(&mut pipes, interval) {
                Ok(_) | Err(Errno::EINTR) => (),
                Err(error) => return Err(error.into()),
            }
        }
    })();
    if result.is_err() {
        let pid = Pid::from_raw(i32::try_from(child.id()).context("invalid child process id")?);
        if let Err(error) = killpg(pid, Signal::SIGKILL) {
            if error != Errno::ESRCH {
                return Err(error.into());
            }
        }
        child.wait()?;
    }
    let output = result?;
    if !output.status.success() {
        bail!(
            "{program} exited {}: {}",
            output.status,
            redact_text(&String::from_utf8_lossy(&output.stderr), Some(1200))
        );
    }
    Ok(output)
}
pub fn available(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|path| {
            use std::os::unix::fs::PermissionsExt;
            let path = path.join(program);
            path.is_file()
                && path
                    .metadata()
                    .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
        })
    })
}
