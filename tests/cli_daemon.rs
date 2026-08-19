use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

fn snap() -> Command {
    Command::new(env!("CARGO_BIN_EXE_snap"))
}

fn wait_status(home: &Path, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        let out = snap().args(["status"]).env("HOME", home).output().unwrap();
        if out.status.success() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn start_status_stop_and_second_start() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    fs::create_dir_all(home.join(".snap")).unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(repo.join(".git")).unwrap();

    let init = snap()
        .args(["init"])
        .env("HOME", &home)
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );

    let start = snap().args(["start"]).env("HOME", &home).output().unwrap();
    assert!(
        start.status.success(),
        "{}",
        String::from_utf8_lossy(&start.stderr)
    );
    assert!(wait_status(&home, Duration::from_secs(3)));

    let status = snap().args(["status"]).env("HOME", &home).output().unwrap();
    assert!(status.status.success());
    let text = String::from_utf8(status.stdout).unwrap();
    assert!(text.contains("pid") || text.contains(&repo.display().to_string()));

    let start2 = snap().args(["start"]).env("HOME", &home).output().unwrap();
    assert!(start2.status.success());
    let msg = format!(
        "{}{}",
        String::from_utf8_lossy(&start2.stdout),
        String::from_utf8_lossy(&start2.stderr)
    );
    assert!(msg.contains("already"), "{msg}");

    let stop = snap().args(["stop"]).env("HOME", &home).output().unwrap();
    assert!(stop.status.success());

    let latest = snap()
        .args(["latest"])
        .env("HOME", &home)
        .current_dir(&repo)
        .output()
        .unwrap();
    assert_eq!(latest.status.code(), Some(1)); // empty inbox, daemon down is fine
}
