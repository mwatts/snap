use std::fs;
use std::path::Path;
use std::process::Command;

fn snap() -> Command {
    Command::new(env!("CARGO_BIN_EXE_snap"))
}

fn home_repo() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(home.join(".snap")).unwrap();
    fs::create_dir_all(repo.join(".git")).unwrap();
    (tmp, home, repo)
}

fn run_ok(home: &Path, repo: &Path, args: &[&str]) -> String {
    let out = snap()
        .args(args)
        .env("HOME", home)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn init_then_latest_empty_then_roundtrip() {
    let (_t, home, repo) = home_repo();
    run_ok(&home, &repo, &["init"]);
    assert!(repo.join(".snap/config.toml").exists());
    assert!(repo.join("AGENTS.md").exists());

    let out = snap()
        .args(["latest"])
        .env("HOME", &home)
        .current_dir(&repo)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["error"], "No annotations in inbox");

    let inbox = repo.join(".snap/inbox");
    fs::write(inbox.join("snap-20260818-142015-897.png"), b"png").unwrap();
    fs::write(
        inbox.join("snap-20260818-142015-897.json"),
        r#"{"timestamp":"t","source":{"window_title":null,"url":null,"window_class":null,"pid":null,"display":"primary","resolution":[1,1]},"annotations":[],"image_filename":"snap-20260818-142015-897.png"}"#,
    )
    .unwrap();

    let latest = run_ok(&home, &repo, &["latest"]);
    let v: serde_json::Value = serde_json::from_str(&latest).unwrap();
    assert_eq!(v["filename"], "snap-20260818-142015-897");
    assert!(v["image_path"].as_str().unwrap().ends_with(".png"));

    let check = run_ok(&home, &repo, &["check"]);
    let c: serde_json::Value = serde_json::from_str(&check).unwrap();
    assert_eq!(c["has_new"], false);

    run_ok(&home, &repo, &["clear"]);
    assert!(!inbox.join("snap-20260818-142015-897.json").exists());
}
