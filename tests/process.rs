use sm::process::run_command;
use std::{process::Command, time::Duration};

#[test]
fn early_child_failure_keeps_its_diagnostic_when_stdin_closes() {
    let mut command = Command::new("sh");
    command.args(["-c", "printf 'analysis startup failed\\n' >&2; exit 7"]);
    let error = run_command(
        command,
        Some(&vec![b'x'; 1_000_000]),
        Duration::from_secs(2),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("analysis startup failed"), "{error}");
    assert!(error.contains("7"), "{error}");
}

#[test]
fn nonzero_exit_cleans_up_detached_output_descendants() {
    let directory = tempfile::tempdir().unwrap();
    let pid_file = directory.path().join("child.pid");
    let mut command = Command::new("sh");
    command
        .args([
            "-c",
            "sleep 30 >/dev/null 2>&1 & printf '%s' \"$!\" > \"$SM_TEST_CHILD_PID\"; exit 7",
        ])
        .env("SM_TEST_CHILD_PID", &pid_file);
    let error = run_command(command, None, Duration::from_secs(2)).unwrap_err();
    assert!(error.to_string().contains("7"));
    let pid: i32 = std::fs::read_to_string(pid_file).unwrap().parse().unwrap();
    let alive = || {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .is_ok_and(|stat| !stat.split_once(") ").unwrap().1.starts_with('Z'))
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while alive() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let leaked = alive();
    if leaked {
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid),
            nix::sys::signal::Signal::SIGKILL,
        )
        .unwrap();
    }
    assert!(
        !leaked,
        "failed installer left a background download running"
    );
}

#[test]
fn failing_command_keeps_stdout_diagnostics_and_redacts_secrets() {
    let mut command = Command::new("sh");
    command.args(["-c", "printf 'installer failed password=private-secret\\n'; printf 'stderr context\\n' >&2; exit 7"]);
    let error = run_command(command, None, Duration::from_secs(2))
        .unwrap_err()
        .to_string();
    assert!(error.contains("installer failed"), "{error}");
    assert!(error.contains("stderr context"), "{error}");
    assert!(!error.contains("private-secret"), "{error}");
    assert!(error.len() < 3000);
}

#[test]
fn timeout_keeps_redacted_stderr_tail_and_deadline_context() {
    let mut command = Command::new("sh");
    command.args([
        "-c",
        "printf 'old diagnostic\\n'; head -c 100000 /dev/zero | tr '\\0' x >&2; printf '\\nlast diagnostic api_key=private-secret Authorization: Bearer private-bearer\\n' >&2; sleep 5",
    ]);
    let error = run_command(command, None, Duration::from_millis(200))
        .unwrap_err()
        .to_string();
    assert!(error.contains("timed out"), "{error}");
    assert!(error.contains("200 ms"), "{error}");
    assert!(error.contains("sh"), "{error}");
    assert!(error.contains("status"), "{error}");
    assert!(error.contains("last diagnostic"), "{error}");
    assert!(error.contains("[redacted]"), "{error}");
    assert!(!error.contains("private-secret"), "{error}");
    assert!(!error.contains("private-bearer"), "{error}");
    assert!(error.len() < 2000, "{} bytes", error.len());
}

#[test]
fn failing_command_retains_final_stderr_without_unbounded_capture() {
    let mut command = Command::new("sh");
    command.args([
        "-c",
        "head -c 100000 /dev/zero | tr '\\0' x >&2; printf '\\nfinal diagnostic password=private-secret\\n' >&2; exit 3",
    ]);
    let error = run_command(command, None, Duration::from_secs(2))
        .unwrap_err()
        .to_string();
    assert!(error.contains("final diagnostic"), "{error}");
    assert!(error.contains("3"), "{error}");
    assert!(!error.contains("private-secret"), "{error}");
    assert!(error.len() < 2000, "{} bytes", error.len());
}

#[test]
fn successful_binary_stdout_is_complete_and_oversized_stdout_fails_explicitly() {
    let mut command = Command::new("head");
    command.args(["-c", "1048576", "/dev/zero"]);
    let output = run_command(command, None, Duration::from_secs(5)).unwrap();
    assert_eq!(output.stdout, vec![0; 1024 * 1024]);
    let mut command = Command::new("head");
    command.args(["-c", "67108865", "/dev/zero"]);
    let error = run_command(command, None, Duration::from_secs(10))
        .unwrap_err()
        .to_string();
    assert!(error.contains("stdout exceeds"), "{error}");
}
