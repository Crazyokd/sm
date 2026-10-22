mod common;
use common::Repo;
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
const EXECUTABLE: &str = env!("CARGO_BIN_EXE_sm");
fn command(repo: &Repo, home: &Path) -> Command {
    let mut command = Command::new(EXECUTABLE);
    command.arg("--repo").arg(&repo.work).env("HOME", home);
    command
}
#[test]
fn native_init_sync_status_and_empty_cycle_use_the_same_commands_without_python() {
    let repo = Repo::new();
    let home = repo.temp.path().join("home");
    let tools = repo.temp.path().join("tools");
    fs::create_dir(&tools).unwrap();
    let systemctl = tools.join("systemctl");
    fs::write(&systemctl, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(systemctl, fs::Permissions::from_mode(0o700)).unwrap();
    let path = format!("{}:{}", tools.display(), std::env::var("PATH").unwrap());
    let output = command(&repo, &home)
        .arg("init")
        .env("PATH", path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["ok"]
            .as_bool()
            .unwrap()
    );
    for subcommand in ["sync", "status", "cycle"] {
        let output = command(&repo, &home).arg(subcommand).output().unwrap();
        assert!(
            output.status.success(),
            "{subcommand}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(serde_json::from_slice::<Value>(&output.stdout).is_ok());
    }
    assert_eq!(
        fs::read_to_string(home.join(".agents/skills/review/SKILL.md")).unwrap(),
        "Approved"
    );
    let unit = fs::read_to_string(home.join(".config/systemd/user/sm-web.service")).unwrap();
    assert!(unit.contains(EXECUTABLE));
    assert!(!unit.contains("python") && !unit.contains("-m sm"));
}
#[test]
fn recorder_is_quiet_nonblocking_and_skips_internal_runs() {
    let root = tempfile::tempdir().unwrap();
    for (input, internal) in [
        ("{\"hook_event_name\":\"Stop\",\"event_id\":\"one\"}", false),
        ("{bad}", false),
        (
            "{\"hook_event_name\":\"Stop\",\"event_id\":\"internal\"}",
            true,
        ),
    ] {
        let mut command = Command::new(EXECUTABLE);
        command
            .args(["record", "--agent", "codex", "--state-dir"])
            .arg(root.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if internal {
            command.env("SM_INTERNAL_RUN", "1");
        }
        let mut child = command.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        assert!(output.stdout.is_empty());
    }
    let lines: usize = fs::read_dir(root.path().join("events"))
        .unwrap()
        .map(|entry| {
            fs::read_to_string(entry.unwrap().path())
                .unwrap()
                .lines()
                .count()
        })
        .sum();
    assert_eq!(lines, 1);
}
#[test]
fn failed_native_proposal_submission_keeps_the_local_draft() {
    let repo = Repo::new();
    let home = repo.temp.path().join("home");
    let input = repo.temp.path().join("change.json");
    fs::write(&input,json!({"resource_ids":["review"],"reason":"Native proposal","writes":{"resources/review/SKILL.md":"Only in draft"},"validation":{"status":"partial"}}).to_string()).unwrap();
    let output = command(&repo, &home)
        .arg("propose")
        .arg(input)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("GitLab"));
    let state = sm::state::State::open(home.join(".local/state/sm/state.sqlite3")).unwrap();
    let proposals: std::collections::BTreeMap<String, sm::proposals::Proposal> =
        state.get("proposals").unwrap();
    assert_eq!(proposals.len(), 1);
    assert_eq!(
        fs::read_to_string(repo.work.join("resources/review/SKILL.md")).unwrap(),
        "Approved"
    );
}
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn http(port: u16) -> String {
    let mut stream = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    write!(
        stream,
        "GET /resources HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}
#[test]
fn actual_server_starts_first_sync_and_serves_embedded_views() {
    let repo = Repo::new();
    let home = repo.temp.path().join("home");
    let mut child = command(&repo, &home)
        .args(["serve", "--port", "0"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = child.stderr.take().unwrap();
    let _running = Running(child);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            match line {
                Ok(line) if line.starts_with("SM: http://") => {
                    let _ = tx.send(line);
                    break;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
    let line = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("native server did not bind");
    let port: u16 = line.rsplit(':').next().unwrap().parse().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let response = http(port);
        assert!(response.starts_with("HTTP/1.1 200"));
        if response.contains("class=\"resource-name\"")
            && home.join(".agents/skills/review/SKILL.md").is_file()
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "initial synchronization did not populate resource rows"
        );
        std::thread::sleep(Duration::from_millis(30));
    }
    assert!(home.join(".agents/skills/review/SKILL.md").is_file());
}
