use crate::ipc::{send_request, Request};
use crate::paths::Paths;
use crate::state::GlobalState;
use std::fs;
use std::process::ExitCode;

pub fn run(paths: &Paths) -> ExitCode {
    let (lines, ok) = collect(paths);
    for line in &lines {
        println!("{line}");
    }
    if ok {
        ExitCode::from(0)
    } else {
        ExitCode::from(1)
    }
}

pub fn report(paths: &Paths) -> Vec<String> {
    collect(paths).0
}

fn collect(paths: &Paths) -> (Vec<String>, bool) {
    let mut lines = Vec::new();
    let mut ok = true;

    match ensure_snap_home(paths) {
        Ok(()) => lines.push(format!("snap_home: ok ({})", paths.snap_home.display())),
        Err(msg) => {
            lines.push(msg);
            ok = false;
        }
    }

    match send_request(&paths.sock, &Request::Ping) {
        Ok(resp) if resp.ok => lines.push("daemon: up".into()),
        _ => {
            lines.push("daemon: down".into());
            ok = false;
        }
    }

    match check_active_project(paths) {
        Ok(line) => lines.push(line),
        Err(line) => {
            lines.push(line);
            ok = false;
        }
    }

    match check_capture_tool() {
        Ok(line) => lines.push(line),
        Err(line) => {
            lines.push(line);
            ok = false;
        }
    }

    #[cfg(target_os = "macos")]
    {
        if screen_recording_ok() {
            lines.push("screen_recording: ok".into());
        } else {
            lines.push(
                "Screen Recording permission is missing. Enable it in System Settings > Privacy & Security > Screen Recording."
                    .into(),
            );
            ok = false;
        }
    }

    if has_old_global_inbox(paths) {
        lines.push(
            "note: old global inbox at ~/.snap/inbox is not used. Per-repo inbox is <repo>/.snap/inbox/"
                .into(),
        );
    }

    (lines, ok)
}

fn ensure_snap_home(paths: &Paths) -> Result<(), String> {
    if paths.snap_home.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(&paths.snap_home).map_err(|e| {
        format!(
            "snap_home: cannot create {}: {e}",
            paths.snap_home.display()
        )
    })
}

fn check_active_project(paths: &Paths) -> Result<String, String> {
    let state = GlobalState::load(&paths.state).map_err(|e| e.to_string())?;
    match state.active_project {
        Some(ref repo) if repo.join(".snap").join("config.toml").is_file() => Ok(format!(
            "active_project: ok ({})",
            repo.display()
        )),
        Some(ref repo) => Err(format!(
            "active_project: {} has no .snap/config.toml — run snap init",
            repo.display()
        )),
        None => Err(
            "active_project: not set — run snap init in a repo, or snap use <path>".into(),
        ),
    }
}

fn check_capture_tool() -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        if command_on_path("screencapture") {
            Ok("capture: screencapture ok".into())
        } else {
            Err("capture: screencapture not found on PATH".into())
        }
    }
    #[cfg(target_os = "linux")]
    {
        for tool in ["gnome-screenshot", "grim", "scrot"] {
            if command_on_path(tool) {
                return Ok(format!("capture: {tool} ok"));
            }
        }
        Err(
            "capture: none of gnome-screenshot, grim, scrot found on PATH".into(),
        )
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err("capture: screen capture is not supported on this OS".into())
    }
}

fn command_on_path(name: &str) -> bool {
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return true;
        }
    }
    false
}

#[cfg(target_os = "macos")]
fn screen_recording_ok() -> bool {
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
    }
    unsafe { CGPreflightScreenCaptureAccess() }
}

fn has_old_global_inbox(paths: &Paths) -> bool {
    let inbox = old_inbox_dir(paths);
    let Ok(entries) = fs::read_dir(&inbox) else {
        return false;
    };
    entries.filter_map(|e| e.ok()).any(|e| {
        e.file_name()
            .to_str()
            .is_some_and(|name| name.starts_with("snap-") && name.ends_with(".json"))
    })
}

fn old_inbox_dir(paths: &Paths) -> std::path::PathBuf {
    // Paths::snap_home is home/.snap; old global inbox lived at home/.snap/inbox.
    paths.snap_home.join("inbox")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::Paths;

    #[test]
    fn doctor_reports_missing_active_project() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::from_snap_home(tmp.path().join(".snap"));
        std::fs::create_dir_all(&paths.snap_home).unwrap();
        let report = crate::doctor::report(&paths);
        assert!(report.iter().any(|l| l.contains("active") || l.contains("snap init")));
    }

    #[test]
    fn doctor_notes_old_global_inbox() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::from_snap_home(tmp.path().join(".snap"));
        std::fs::create_dir_all(&paths.snap_home).unwrap();
        let inbox = paths.snap_home.join("inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        std::fs::write(inbox.join("snap-20260818-120000-001.json"), "{}").unwrap();
        let report = crate::doctor::report(&paths);
        assert!(
            report.iter().any(|l| l.contains("old global inbox")),
            "{report:?}"
        );
    }

    #[test]
    fn doctor_ok_active_when_config_present() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::from_snap_home(tmp.path().join(".snap"));
        std::fs::create_dir_all(&paths.snap_home).unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(repo.join(".snap")).unwrap();
        std::fs::write(repo.join(".snap/config.toml"), "[project]\nname = \"t\"\n").unwrap();
        GlobalState {
            active_project: Some(repo.clone()),
        }
        .save(&paths.state)
        .unwrap();
        let report = crate::doctor::report(&paths);
        assert!(
            report.iter().any(|l| l.contains("active_project: ok")),
            "{report:?}"
        );
    }
}
