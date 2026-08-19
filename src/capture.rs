use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Failed(String),
}

pub trait ScreenCapture {
    fn capture_to(&self, path: &Path) -> Result<(), CaptureError>;
}

pub fn capture_temp_path() -> PathBuf {
    std::env::temp_dir().join("snap-capture.png")
}

pub struct CliCapture;

impl ScreenCapture for CliCapture {
    fn capture_to(&self, path: &Path) -> Result<(), CaptureError> {
        #[cfg(target_os = "macos")]
        {
            macos_capture(path)
        }
        #[cfg(target_os = "linux")]
        {
            linux_capture(path)
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _ = path;
            Err(CaptureError::Failed(
                "screen capture is not supported on this OS".into(),
            ))
        }
    }
}

#[cfg(target_os = "macos")]
fn macos_capture(path: &Path) -> Result<(), CaptureError> {
    let _ = fs::remove_file(path);
    let path_str = path.to_str().unwrap_or("/tmp/snap-capture.png");
    let output = Command::new("screencapture")
        .args(["-i", "-s", path_str])
        .output()
        .map_err(|e| CaptureError::Failed(format!("screencapture failed to launch: {e}")))?;
    finish_capture(path, "screencapture", &output)
}

#[cfg(target_os = "linux")]
fn linux_capture(path: &Path) -> Result<(), CaptureError> {
    let _ = fs::remove_file(path);
    let path_str = path.to_string_lossy().into_owned();
    let methods: [(&str, Vec<&str>); 3] = [
        ("gnome-screenshot", vec!["--file", &path_str]),
        ("grim", vec![&path_str]),
        ("scrot", vec!["--overwrite", &path_str]),
    ];
    let mut last_error = String::from("No screenshot tool found");
    for (tool, args) in &methods {
        match Command::new(tool).args(args).output() {
            Ok(output) => match finish_capture(path, tool, &output) {
                Ok(()) => return Ok(()),
                Err(CaptureError::Cancelled) => {
                    last_error = format!("{tool} produced no file");
                }
                Err(CaptureError::Failed(msg)) => last_error = msg,
            },
            Err(_) => continue,
        }
    }
    Err(CaptureError::Failed(format!(
        "Screen capture failed: {last_error}. Install one of: sudo apt install gnome-screenshot grim scrot"
    )))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn finish_capture(
    path: &Path,
    tool: &str,
    output: &std::process::Output,
) -> Result<(), CaptureError> {
    if !output.status.success() {
        if !path.exists() {
            return Err(CaptureError::Cancelled);
        }
        return Err(CaptureError::Failed(format!(
            "{tool} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    match fs::metadata(path) {
        Ok(meta) if meta.len() > 0 => Ok(()),
        Ok(_) => Err(CaptureError::Failed(format!("{tool} produced empty file"))),
        Err(_) => Err(CaptureError::Cancelled),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_trait_fake_writes_file() {
        struct Fake;
        impl ScreenCapture for Fake {
            fn capture_to(&self, path: &std::path::Path) -> Result<(), CaptureError> {
                std::fs::write(path, b"ok").map_err(|e| CaptureError::Failed(e.to_string()))
            }
        }
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("c.png");
        Fake.capture_to(&p).unwrap();
        assert_eq!(std::fs::read(p).unwrap(), b"ok");
    }

    #[test]
    fn capture_temp_path_is_snap_capture_png() {
        assert_eq!(capture_temp_path().file_name().unwrap(), "snap-capture.png");
    }
}
