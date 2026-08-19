# Snap Rust Rewrite Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the Tauri + Python MCP fork with one `snap` binary that is the daemon, the agent CLI, project init, and the GPUI overlay.

**Architecture:** One crate at the repo root. Library modules own inbox, project registry, IPC, draw, and capture. `main.rs` only parses CLI and dispatches. Agent commands read `<repo>/.snap/inbox/` on disk. `snap daemon` is one process: a socket thread plus (from Task 11) a GPUI event loop. Last-active project in `~/.snap/state.toml` selects the inbox for a hotkey save.

**Tech Stack:** Rust edition 2024, clap, serde/serde_json, toml, thiserror, anyhow, tracing, dirs, chrono, tempfile (dev), tiny-skia, fontdue, global-hotkey, gpui + gpui_platform (git pin), PNG encode via `png` crate.

**Spec:** `docs/superpowers/specs/2026-08-18-rust-rewrite-design.md`

## Global Constraints

- One crate, one binary named `snap`. No workspace. Do not add `app/src-tauri` to a workspace.
- Do not delete `app/`, `mcp-server/`, or `setup-mcp.sh` until Task 13.
- Edition 2024. `rust-version = "1.85"`.
- Application edge uses `anyhow`. Matchable errors use `thiserror` (`InboxError`, `IpcError`, `ResolveError`, `StateError`).
- Agent commands print JSON on stdout. Errors also print one line on stderr. Exit 0 success, 1 user/environment, 2 usage.
- Inbox is `<repo>/.snap/inbox/`. Never write `~/.snap/inbox/`.
- Agent commands MUST work when the daemon is down.
- `snap get` does not write `last_read`. `snap latest` and `snap list` do. `snap check` does not.
- Coordinates in the sidecar are overlay logical pixels. Crop scale matches `app/src/export-scale.mjs`.
- Sidecar `type` tag and field names match the fork (see Task 2).
- Freehand `points` are objects `{ "x": n, "y": n }`, not tuples.
- Tests use a temp `HOME`. Do not touch the developer's real `~/.snap`.
- v1 tests are `cargo test` with no GPU. Do not add `#[gpui::test]` as a gate.
- macOS first. Linux capture adapters land in Task 10. Windows overlay is out of v1.
- Log to `~/.snap/snap.log`. Rotate at 1_000_000 bytes to `snap.log.old`. No image bytes in the log.
- Commits: short, direct messages.

---

## File map

| Path | Responsibility |
|------|----------------|
| `Cargo.toml` | Package `snap`, binary + lib, dependencies |
| `src/lib.rs` | Module tree only |
| `src/main.rs` | `fn main()` → `snap::cli::run()` |
| `src/paths.rs` | `Paths` for `~/.snap` and the Unix socket |
| `src/logutil.rs` | `tracing` file writer + 1 MB rotate |
| `src/schema.rs` | `Annotation`, `Point`, `WindowSource`, `Sidecar`, `AnnotationRecord`, `CheckResult` |
| `src/inbox.rs` | Read/write/list/get/check/clear/save pairs |
| `src/state.rs` | `state.toml`, `projects.toml` |
| `src/project.rs` | git root, resolve, init, markdown block |
| `src/cli.rs` | clap + dispatch + exit codes |
| `src/ipc.rs` | Request/Response + line codec + `send_request` |
| `src/daemon.rs` | Single-instance socket server; later GPUI run |
| `src/draw.rs` | Tools, session, undo, simplify, crop map, composite |
| `src/capture.rs` | `ScreenCapture` trait + macOS/Linux adapters |
| `src/window.rs` | `WindowContext` trait + macOS/Linux adapters |
| `src/overlay.rs` | GPUI window, toolbar, project chip, keys |
| `src/doctor.rs` | `snap doctor` checks |
| `assets/LiberationSans-Bold.ttf` | Bundled export font |
| `tests/cli_agent.rs` | Binary tests for init/latest/check/clear |
| `tests/cli_daemon.rs` | Binary tests for start/status/stop |
| `snap.plist`, `snap.service`, `Makefile`, `README.md`, `SETUP.md`, `CLAUDE.md` | Rewritten in Task 13 |

Do not create extra crates. Do not add Node or Python.

---

## Shared types (lock these names)

Every later task uses these exact signatures. Do not rename them.

```rust
// src/paths.rs
pub struct Paths {
    pub snap_home: PathBuf,
    pub sock: PathBuf,
    pub pid: PathBuf,
    pub log: PathBuf,
    pub state: PathBuf,
    pub projects: PathBuf,
}
impl Paths {
    pub fn discover() -> Result<Self, anyhow::Error>;
    pub fn from_snap_home(snap_home: PathBuf) -> Self;
}

// src/schema.rs
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Point { pub x: f64, pub y: f64 }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowSource {
    pub window_title: Option<String>,
    pub url: Option<String>,
    pub window_class: Option<String>,
    pub pid: Option<u32>,
    pub display: String,
    pub resolution: [u32; 2],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Annotation {
    #[serde(rename = "circle")]
    Circle { center: [f64; 2], radius: [f64; 2], color: String, label: Option<String> },
    #[serde(rename = "rect")]
    Rect { position: [f64; 2], size: [f64; 2], color: String, label: Option<String> },
    #[serde(rename = "arrow")]
    Arrow { from: [f64; 2], to: [f64; 2], color: String, label: Option<String> },
    #[serde(rename = "freehand")]
    Freehand { points: Vec<Point>, color: String, label: Option<String> },
    #[serde(rename = "text")]
    Text { position: [f64; 2], content: String, color: String },
    #[serde(rename = "marker")]
    Marker { position: [f64; 2], number: u32, color: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sidecar {
    pub timestamp: String,
    pub source: WindowSource,
    pub annotations: Vec<Annotation>,
    pub image_filename: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnnotationRecord {
    pub filename: String,
    pub image_path: Option<String>,
    pub timestamp: String,
    pub source: WindowSource,
    pub annotations: Vec<Annotation>,
    pub image_filename: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckResult {
    pub has_new: bool,
    pub new_count: usize,
    pub new_annotations: Vec<String>,
    pub total_in_inbox: usize,
}

// src/inbox.rs
pub struct Inbox { pub snap_dir: PathBuf } // the <repo>/.snap directory
impl Inbox {
    pub fn new(snap_dir: PathBuf) -> Self;
    pub fn latest(&self, mark_read: bool) -> Result<AnnotationRecord, InboxError>;
    pub fn list(&self, last_n: usize, mark_read: bool) -> Result<Vec<AnnotationRecord>, InboxError>;
    pub fn get(&self, id: &str) -> Result<AnnotationRecord, InboxError>;
    pub fn check(&self) -> Result<CheckResult, InboxError>;
    pub fn clear(&self) -> Result<ClearResult, InboxError>;
    pub fn save(&self, sidecar: &mut Sidecar, png: &[u8], now: DateTime<Utc>) -> Result<PathBuf, InboxError>;
}

#[derive(Debug, thiserror::Error)]
pub enum InboxError {
    #[error("No annotations in inbox")]
    Empty,
    #[error("Annotation '{0}' not found")]
    NotFound(String),
    #[error("corrupt or unreadable: {0}")]
    Corrupt(String),
    #[error("{0}")]
    Io(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClearResult {
    pub deleted_json: usize,
    pub deleted_png: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

// src/state.rs
pub struct GlobalState { pub active_project: Option<PathBuf> }
pub struct ProjectEntry { pub path: PathBuf, pub name: String, pub inited_at: String }
pub struct ProjectRegistry { pub projects: Vec<ProjectEntry> }

// src/project.rs
pub const AGENT_BLOCK_START: &str = "<!-- snap-agent-block:start -->";
pub const AGENT_BLOCK_END: &str = "<!-- snap-agent-block:end -->";
pub fn find_git_root(start: &Path) -> Option<PathBuf>;
pub fn resolve_repo(cwd: &Path, paths: &Paths) -> Result<PathBuf, ResolveError>;
pub fn init_project(cwd: &Path, paths: &Paths, now: DateTime<Utc>) -> Result<PathBuf, InitError>;
pub fn set_active(paths: &Paths, repo: &Path) -> Result<(), StateError>;
pub fn upsert_project(paths: &Paths, repo: &Path, name: &str, now: DateTime<Utc>) -> Result<(), StateError>;
pub fn patch_agent_markdown(repo: &Path) -> Result<PathBuf, InitError>;
pub fn agent_markdown_block() -> String;

// src/ipc.rs
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Request {
    Ping,
    Capture,
    Use { path: String },
    Status,
    Shutdown,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
impl Response {
    pub fn ok(data: serde_json::Value) -> Self;
    pub fn err(msg: impl Into<String>) -> Self;
}
pub fn send_request(sock: &Path, req: &Request) -> Result<Response, IpcError>;

// src/draw.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool { Circle, Rect, Arrow, Freehand, Text, Marker }

#[derive(Debug, Clone, PartialEq)]
pub struct Crop { pub x: f64, pub y: f64, pub w: f64, pub h: f64 }

#[derive(Debug, Clone, PartialEq)]
pub struct CropDims {
    pub preview_width: f64,
    pub preview_height: f64,
    pub native_width: f64,
    pub native_height: f64,
}

pub fn map_crop_to_native(crop: &Crop, dims: &CropDims) -> Option<Crop>;
pub fn simplify_points(points: &[Point], tolerance: f64) -> Vec<Point>;

pub struct Session {
    pub tool: Tool,
    pub color: String,
    pub width: f32,
    pub dim: bool,
    pub marker_counter: u32,
    pub items: Vec<SessionItem>,
}
pub enum SessionItem { /* internal live shapes; convert via Session::to_annotations */ }
impl Session {
    pub fn new() -> Self;
    pub fn undo(&mut self);
    pub fn clear(&mut self);
    pub fn add_marker(&mut self, x: f64, y: f64);
    pub fn to_annotations(&self) -> Vec<Annotation>;
}

pub fn composite_png(
    capture_png: &[u8],
    crop: Option<&Crop>,
    annotations: &[Annotation],
    logical_w: f64,
    logical_h: f64,
    font: &[u8],
) -> Result<Vec<u8>, DrawError>;

// src/capture.rs
pub trait ScreenCapture {
    fn capture_to(&self, path: &Path) -> Result<(), CaptureError>;
}

// src/window.rs
pub trait ActiveWindow {
    fn read(&self) -> WindowSource;
}
```

---

### Task 1: Crate, Paths, log rotate

**Files:**
- Create: `Cargo.toml`
- Create: `src/lib.rs`
- Create: `src/main.rs`
- Create: `src/paths.rs` (unit tests in the same file)
- Create: `src/logutil.rs` (unit tests in the same file)

**Interfaces:**
- Consumes: nothing
- Produces: `Paths::discover`, `Paths::from_snap_home`, `logutil::init(paths: &Paths) -> Result<(), anyhow::Error>`, `logutil::LOG_ROTATE_BYTES = 1_000_000`

- [ ] **Step 1: Write the failing tests**

In `src/paths.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn from_snap_home_sets_child_paths() {
        let p = Paths::from_snap_home(PathBuf::from("/tmp/snap-home"));
        assert_eq!(p.snap_home, PathBuf::from("/tmp/snap-home"));
        assert_eq!(p.pid, PathBuf::from("/tmp/snap-home/snap.pid"));
        assert_eq!(p.log, PathBuf::from("/tmp/snap-home/snap.log"));
        assert_eq!(p.state, PathBuf::from("/tmp/snap-home/state.toml"));
        assert_eq!(p.projects, PathBuf::from("/tmp/snap-home/projects.toml"));
        assert_eq!(p.sock.file_name().unwrap(), "snap.sock");
    }

    #[test]
    fn discover_uses_home_join_snap() {
        let tmp = tempfile::tempdir().unwrap();
        let prev = std::env::var_os("HOME");
        // SAFETY: test process, HOME restored below
        unsafe { std::env::set_var("HOME", tmp.path()); }
        let p = Paths::discover().unwrap();
        assert_eq!(p.snap_home, tmp.path().join(".snap"));
        match prev {
            Some(v) => unsafe { std::env::set_var("HOME", v); }
            None => unsafe { std::env::remove_var("HOME"); }
        }
    }
}
```

In `src/logutil.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::Paths;
    use std::fs;
    use std::io::Write;

    #[test]
    fn rotate_renames_when_over_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::from_snap_home(tmp.path().to_path_buf());
        fs::create_dir_all(&paths.snap_home).unwrap();
        let mut f = fs::File::create(&paths.log).unwrap();
        f.write_all(&vec![b'x'; LOG_ROTATE_BYTES + 1]).unwrap();
        drop(f);
        rotate_if_needed(&paths).unwrap();
        assert!(paths.snap_home.join("snap.log.old").exists());
        assert!(!paths.log.exists() || fs::metadata(&paths.log).unwrap().len() == 0);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib paths::tests::from_snap_home_sets_child_paths logutil::tests::rotate_renames_when_over_limit`
Expected: FAIL — `src/lib.rs` / modules do not exist, or `Paths` is missing.

- [ ] **Step 3: Write minimal implementation**

`Cargo.toml`:

```toml
[package]
name = "snap"
version = "0.1.0"
edition = "2024"
rust-version = "1.85"
license = "MIT"
description = "Screen annotation for agents: daemon, CLI, and overlay"

[dependencies]
anyhow = "1"
chrono = { version = "0.4", default-features = false, features = ["clock", "std"] }
clap = { version = "4", features = ["derive"] }
dirs = "6"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
toml = "0.8"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["fmt"] }

[dev-dependencies]
tempfile = "3"

[lints.rust]
unsafe_op_in_unsafe_fn = "warn"
```

Do **not** add gpui, tiny-skia, or global-hotkey yet.

`src/lib.rs`:

```rust
pub mod logutil;
pub mod paths;
```

`src/main.rs`:

```rust
fn main() {
    println!("snap");
}
```

Task 6 replaces this `main` with `snap::cli::run()`.

`src/paths.rs` — implement `from_snap_home` so `sock` is `snap_home.join("snap.sock")` on macOS. On Linux, if `XDG_RUNTIME_DIR` is set, `sock` is `PathBuf::from(xdg).join("snap.sock")`, else `snap_home.join("snap.sock")`.

```rust
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Paths {
    pub snap_home: PathBuf,
    pub sock: PathBuf,
    pub pid: PathBuf,
    pub log: PathBuf,
    pub state: PathBuf,
    pub projects: PathBuf,
}

impl Paths {
    pub fn from_snap_home(snap_home: PathBuf) -> Self {
        let sock = linux_sock().unwrap_or_else(|| snap_home.join("snap.sock"));
        Self {
            pid: snap_home.join("snap.pid"),
            log: snap_home.join("snap.log"),
            state: snap_home.join("state.toml"),
            projects: snap_home.join("projects.toml"),
            snap_home,
            sock,
        }
    }

    pub fn discover() -> Result<Self, anyhow::Error> {
        let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
        Ok(Self::from_snap_home(home.join(".snap")))
    }
}

fn linux_sock() -> Option<PathBuf> {
    if cfg!(target_os = "linux") {
        let xdg = std::env::var_os("XDG_RUNTIME_DIR")?;
        if xdg.is_empty() {
            return None;
        }
        Some(PathBuf::from(xdg).join("snap.sock"))
    } else {
        None
    }
}
```

`src/logutil.rs`:

```rust
use crate::paths::Paths;
use std::fs;
use std::io::Write;

pub const LOG_ROTATE_BYTES: u64 = 1_000_000;

pub fn rotate_if_needed(paths: &Paths) -> Result<(), anyhow::Error> {
    if let Ok(meta) = fs::metadata(&paths.log) {
        if meta.len() > LOG_ROTATE_BYTES {
            let backup = paths.snap_home.join("snap.log.old");
            fs::rename(&paths.log, &backup)?;
        }
    }
    Ok(())
}

pub fn init(paths: &Paths) -> Result<(), anyhow::Error> {
    fs::create_dir_all(&paths.snap_home)?;
    rotate_if_needed(paths)?;
    Ok(())
}

pub fn log_event(paths: &Paths, msg: &str) {
    let _ = (|| -> Result<(), anyhow::Error> {
        fs::create_dir_all(&paths.snap_home)?;
        rotate_if_needed(paths)?;
        let ts = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S");
        let line = format!("[{ts}] {msg}\n");
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&paths.log)?
            .write_all(line.as_bytes())?;
        Ok(())
    })();
}
```

On Linux, `from_snap_home_sets_child_paths` may fail if `XDG_RUNTIME_DIR` is set. In that test, unset `XDG_RUNTIME_DIR` for the assertion on `sock` **or** assert only `file_name() == "snap.sock"` (already in the test). The test above already only checks the socket file name. Good.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/lib.rs src/main.rs src/paths.rs src/logutil.rs
git commit -m "feat: add snap crate with paths and log rotate"
```

---

### Task 2: Sidecar schema

**Files:**
- Create: `src/schema.rs`
- Modify: `src/lib.rs` — add `pub mod schema;`

**Interfaces:**
- Consumes: nothing
- Produces: `Point`, `WindowSource`, `Annotation`, `Sidecar`, `AnnotationRecord`, `CheckResult` exactly as in Shared types

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circle_round_trip_matches_fork_shape() {
        let json = r#"{
            "timestamp": "2026-08-18T14:20:15.897Z",
            "source": {
                "window_title": "localhost:3000",
                "url": null,
                "window_class": "Brave-browser",
                "pid": 12345,
                "display": "primary",
                "resolution": [1920, 1080]
            },
            "annotations": [
                {
                    "type": "circle",
                    "center": [340.0, 220.0],
                    "radius": [45.0, 45.0],
                    "color": "#FF3B30",
                    "label": null
                },
                {
                    "type": "freehand",
                    "points": [{"x": 1.0, "y": 2.0}, {"x": 3.0, "y": 4.0}],
                    "color": "#007AFF",
                    "label": null
                },
                {
                    "type": "text",
                    "position": [10.0, 20.0],
                    "content": "fix this",
                    "color": "#FF3B30"
                },
                {
                    "type": "marker",
                    "position": [5.0, 6.0],
                    "number": 1,
                    "color": "#34C759"
                }
            ],
            "image_filename": "snap-20260818-142015-897.png"
        }"#;
        let side: Sidecar = serde_json::from_str(json).unwrap();
        assert_eq!(side.annotations.len(), 4);
        let out = serde_json::to_value(&side).unwrap();
        assert_eq!(out["annotations"][0]["type"], "circle");
        assert_eq!(out["annotations"][1]["points"][0]["x"], 1.0);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib schema::tests::circle_round_trip_matches_fork_shape`
Expected: FAIL — `schema` module missing

- [ ] **Step 3: Write minimal implementation**

Put the Shared types structs in `src/schema.rs` with the serde attributes shown. Implement `Default` for `WindowSource`:

```rust
impl Default for WindowSource {
    fn default() -> Self {
        Self {
            window_title: None,
            url: None,
            window_class: None,
            pid: None,
            display: "primary".into(),
            resolution: [0, 0],
        }
    }
}
```

`AnnotationRecord::from_sidecar(sidecar: Sidecar, image_path: Option<String>) -> Self` is allowed as a helper:

```rust
impl AnnotationRecord {
    pub fn from_sidecar(sidecar: Sidecar, image_path: Option<String>) -> Self {
        Self {
            filename: sidecar.image_filename.trim_end_matches(".png").to_string(),
            image_path,
            timestamp: sidecar.timestamp,
            source: sidecar.source,
            annotations: sidecar.annotations,
            image_filename: sidecar.image_filename,
            warning: None,
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib schema`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/schema.rs src/lib.rs
git commit -m "feat: add annotation sidecar schema"
```

---

### Task 3: Inbox

**Files:**
- Create: `src/inbox.rs`
- Modify: `src/lib.rs` — add `pub mod inbox;`

**Interfaces:**
- Consumes: `Sidecar`, `AnnotationRecord`, `CheckResult` from `schema`
- Produces: `Inbox`, `InboxError`, `ClearResult`, methods in Shared types. `save` names the pair `snap-{YYYYMMDD}-{HHMMSS}-{mmm}` using `now`, writes PNG bytes, sets `sidecar.image_filename`, writes pretty JSON.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Annotation, Sidecar, WindowSource};
    use chrono::{TimeZone, Utc};
    use std::fs;

    fn inbox_tmp() -> (tempfile::TempDir, Inbox) {
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join(".snap");
        fs::create_dir_all(snap.join("inbox")).unwrap();
        (tmp, Inbox::new(snap))
    }

    fn side(name: &str) -> Sidecar {
        Sidecar {
            timestamp: "2026-08-18T14:20:15.897Z".into(),
            source: WindowSource::default(),
            annotations: vec![Annotation::Circle {
                center: [1.0, 2.0],
                radius: [3.0, 4.0],
                color: "#FF3B30".into(),
                label: None,
            }],
            image_filename: name.into(),
        }
    }

    #[test]
    fn latest_empty_is_error() {
        let (_t, inbox) = inbox_tmp();
        let err = inbox.latest(false).unwrap_err();
        assert!(matches!(err, InboxError::Empty));
        assert_eq!(err.to_string(), "No annotations in inbox");
    }

    #[test]
    fn save_then_latest_round_trips() {
        let (_t, inbox) = inbox_tmp();
        let mut s = side("ignore.png");
        let now = Utc.with_ymd_and_hms(2026, 8, 18, 14, 20, 15).unwrap()
            + chrono::TimeDelta::milliseconds(897);
        let png_path = inbox.save(&mut s, b"\x89PNG-bytes", now).unwrap();
        assert!(png_path.ends_with("snap-20260818-142015-897.png"));
        let rec = inbox.latest(true).unwrap();
        assert_eq!(rec.filename, "snap-20260818-142015-897");
        assert!(rec.image_path.unwrap().ends_with(".png"));
        assert_eq!(rec.annotations.len(), 1);
        assert!(inbox.snap_dir.join("last_read").exists());
    }

    #[test]
    fn get_missing_is_not_found() {
        let (_t, inbox) = inbox_tmp();
        let err = inbox.get("snap-nope").unwrap_err();
        assert!(matches!(err, InboxError::NotFound(_)));
        assert_eq!(err.to_string(), "Annotation 'snap-nope' not found");
    }

    #[test]
    fn get_does_not_write_last_read() {
        let (_t, inbox) = inbox_tmp();
        let mut s = side("ignore.png");
        inbox.save(&mut s, b"png", Utc::now()).unwrap();
        let stem = s.image_filename.trim_end_matches(".png").to_string();
        inbox.get(&stem).unwrap();
        assert!(!inbox.snap_dir.join("last_read").exists());
    }

    #[test]
    fn check_sees_new_until_latest_marks() {
        let (_t, inbox) = inbox_tmp();
        let mut s = side("ignore.png");
        inbox.save(&mut s, b"png", Utc::now()).unwrap();
        let c1 = inbox.check().unwrap();
        assert_eq!(c1.new_count, 1);
        assert!(c1.has_new);
        inbox.latest(true).unwrap();
        let c2 = inbox.check().unwrap();
        assert_eq!(c2.new_count, 0);
        assert!(!c2.has_new);
        assert_eq!(c2.total_in_inbox, 1);
    }

    #[test]
    fn list_newest_first_and_limit() {
        let (_t, inbox) = inbox_tmp();
        inbox.save(&mut side("a.png"), b"1", Utc::now()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        inbox.save(&mut side("b.png"), b"2", Utc::now()).unwrap();
        let all = inbox.list(5, false).unwrap();
        assert_eq!(all.len(), 2);
        let one = inbox.list(1, false).unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].filename, all[0].filename);
    }

    #[test]
    fn clear_removes_pairs_and_last_read() {
        let (_t, inbox) = inbox_tmp();
        inbox.save(&mut side("a.png"), b"1", Utc::now()).unwrap();
        inbox.latest(true).unwrap();
        let r = inbox.clear().unwrap();
        assert_eq!(r.deleted_json, 1);
        assert_eq!(r.deleted_png, 1);
        assert!(inbox.latest(false).is_err());
        assert!(!inbox.snap_dir.join("last_read").exists());
    }

    #[test]
    fn missing_png_sets_warning() {
        let (_t, inbox) = inbox_tmp();
        let json = inbox.snap_dir.join("inbox").join("snap-orphan.json");
        fs::write(&json, serde_json::to_string(&side("snap-orphan.png")).unwrap()).unwrap();
        let rec = inbox.get("snap-orphan").unwrap();
        assert_eq!(rec.image_path, None);
        assert_eq!(rec.warning.as_deref(), Some("image file missing"));
    }
}
```

Do not add a `filetime` crate. Use `Inbox::save` for every pair in these tests.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib inbox`
Expected: FAIL — `inbox` module missing

- [ ] **Step 3: Write minimal implementation**

```rust
impl Inbox {
    pub fn new(snap_dir: PathBuf) -> Self { Self { snap_dir } }

    fn inbox_dir(&self) -> PathBuf { self.snap_dir.join("inbox") }

    fn json_files_newest_first(&self) -> Result<Vec<PathBuf>, InboxError> {
        let dir = self.inbox_dir();
        if !dir.exists() {
            return Ok(vec![]);
        }
        let mut files: Vec<_> = fs::read_dir(&dir)
            .map_err(|e| InboxError::Io(e.to_string()))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension().and_then(|s| s.to_str()) == Some("json")
                    && p.file_name()
                        .and_then(|s| s.to_str())
                        .is_some_and(|n| n.starts_with("snap-"))
            })
            .collect();
        files.sort_by(|a, b| {
            let ma = fs::metadata(a).and_then(|m| m.modified()).ok();
            let mb = fs::metadata(b).and_then(|m| m.modified()).ok();
            mb.cmp(&ma).then_with(|| b.cmp(a))
        });
        Ok(files)
    }

    fn load_path(&self, json_path: &Path) -> Result<AnnotationRecord, InboxError> {
        let text = fs::read_to_string(json_path).map_err(|e| InboxError::Corrupt(e.to_string()))?;
        let sidecar: Sidecar = serde_json::from_str(&text).map_err(|e| InboxError::Corrupt(e.to_string()))?;
        let png = json_path.with_extension("png");
        let (image_path, warning) = if png.exists() {
            (Some(png.to_string_lossy().into_owned()), None)
        } else {
            (None, Some("image file missing".into()))
        };
        let mut rec = AnnotationRecord::from_sidecar(sidecar, image_path);
        rec.filename = json_path
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        rec.warning = warning;
        Ok(rec)
    }

    fn last_read(&self) -> f64 {
        fs::read_to_string(self.snap_dir.join("last_read"))
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0.0)
    }

    fn mark_read(&self) -> Result<(), InboxError> {
        fs::create_dir_all(&self.snap_dir).map_err(|e| InboxError::Io(e.to_string()))?;
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        fs::write(self.snap_dir.join("last_read"), format!("{secs}"))
            .map_err(|e| InboxError::Io(e.to_string()))
    }

    pub fn latest(&self, mark_read: bool) -> Result<AnnotationRecord, InboxError> {
        let files = self.json_files_newest_first()?;
        let Some(p) = files.first() else { return Err(InboxError::Empty) };
        let rec = self.load_path(p)?;
        if mark_read {
            self.mark_read()?;
        }
        Ok(rec)
    }

    pub fn list(&self, last_n: usize, mark_read: bool) -> Result<Vec<AnnotationRecord>, InboxError> {
        let files = self.json_files_newest_first()?;
        let recs = files.iter().take(last_n).map(|p| self.load_path(p)).collect::<Result<Vec<_>, _>>()?;
        if mark_read && !recs.is_empty() {
            self.mark_read()?;
        }
        Ok(recs)
    }

    pub fn get(&self, id: &str) -> Result<AnnotationRecord, InboxError> {
        let json = self.inbox_dir().join(format!("{id}.json"));
        if !json.exists() {
            return Err(InboxError::NotFound(id.into()));
        }
        self.load_path(&json)
    }

    pub fn check(&self) -> Result<CheckResult, InboxError> {
        let files = self.json_files_newest_first()?;
        let last = self.last_read();
        let mut new_annotations = Vec::new();
        for p in &files {
            let mtime = fs::metadata(p)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            if mtime > last {
                if let Some(stem) = p.file_stem() {
                    new_annotations.push(stem.to_string_lossy().into_owned());
                }
            }
        }
        Ok(CheckResult {
            has_new: !new_annotations.is_empty(),
            new_count: new_annotations.len(),
            total_in_inbox: files.len(),
            new_annotations,
        })
    }

    pub fn clear(&self) -> Result<ClearResult, InboxError> {
        let files = self.json_files_newest_first()?;
        let mut deleted_json = 0;
        let mut deleted_png = 0;
        let mut errors = Vec::new();
        for json in files {
            let png = json.with_extension("png");
            if png.exists() {
                match fs::remove_file(&png) {
                    Ok(()) => deleted_png += 1,
                    Err(e) => errors.push(format!("{}: {e}", png.display())),
                }
            }
            match fs::remove_file(&json) {
                Ok(()) => deleted_json += 1,
                Err(e) => errors.push(format!("{}: {e}", json.display())),
            }
        }
        let _ = fs::remove_file(self.snap_dir.join("last_read"));
        Ok(ClearResult { deleted_json, deleted_png, errors })
    }

    pub fn save(
        &self,
        sidecar: &mut Sidecar,
        png: &[u8],
        now: DateTime<Utc>,
    ) -> Result<PathBuf, InboxError> {
        let dir = self.inbox_dir();
        fs::create_dir_all(&dir).map_err(|e| InboxError::Io(e.to_string()))?;
        let stem = now.format("%Y%m%d-%H%M%S-%3f").to_string().replace('.', "-");
        let stem = format!("snap-{stem}");
        let png_name = format!("{stem}.png");
        let png_path = dir.join(&png_name);
        let json_path = dir.join(format!("{stem}.json"));
        fs::write(&png_path, png).map_err(|e| InboxError::Io(e.to_string()))?;
        sidecar.image_filename = png_name;
        let text = serde_json::to_string_pretty(sidecar).map_err(|e| InboxError::Corrupt(e.to_string()))?;
        fs::write(&json_path, text).map_err(|e| InboxError::Io(e.to_string()))?;
        Ok(png_path)
    }
}
```

Chrono `%3f` is fractional seconds. After `replace('.', "-")`, `14:20:15.897` in the time portion of `%H%M%S-%3f` becomes `142015-897`. Combined with `%Y%m%d-` this yields `snap-20260818-142015-897`. If a chrono version prints `%3f` without a leading dot in this format string, adjust the format to `"%Y%m%d-%H%M%S%.3f"` and then `replace('.', "-")` (this is what `app/src-tauri/src/lib.rs` does). Prefer:

```rust
let stem = now.format("%Y%m%d-%H%M%S%.3f").to_string().replace('.', "-");
let stem = format!("snap-{stem}");
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib inbox`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/inbox.rs src/lib.rs
git commit -m "feat: add per-repo inbox read and write"
```

---

### Task 4: Project registry and resolve

**Files:**
- Create: `src/state.rs`
- Create: `src/project.rs`
- Modify: `src/lib.rs` — add `pub mod state;` and `pub mod project;`

**Interfaces:**
- Consumes: `Paths`
- Produces: `GlobalState`, `ProjectRegistry`, `ProjectEntry`, `find_git_root`, `resolve_repo`, `set_active`, `upsert_project`, `ResolveError`, `StateError`

`resolve_repo` algorithm:
1. Walk `cwd` and parents. Return the first dir that contains `.snap/config.toml`.
2. Else load `paths.state`. If `active_project` is a dir that contains `.snap/config.toml`, return it.
3. Else `ResolveError::NoProject`.

`ResolveError` display MUST contain both `snap init` and `snap use`.

- [ ] **Step 1: Write the failing tests**

```rust
// in src/project.rs
#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::Paths;
    use chrono::Utc;
    use std::fs;

    fn setup() -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::from_snap_home(tmp.path().join("global"));
        fs::create_dir_all(&paths.snap_home).unwrap();
        (tmp, paths)
    }

    #[test]
    fn find_git_root_walks_up() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join(".git")).unwrap();
        let nested = tmp.path().join("a/b");
        fs::create_dir_all(&nested).unwrap();
        assert_eq!(find_git_root(&nested).unwrap(), tmp.path());
    }

    #[test]
    fn resolve_prefers_cwd_snap_over_active() {
        let (tmp, paths) = setup();
        let repo_a = tmp.path().join("a");
        let repo_b = tmp.path().join("b");
        fs::create_dir_all(repo_a.join(".snap")).unwrap();
        fs::write(repo_a.join(".snap/config.toml"), "[project]\nname = \"a\"\n").unwrap();
        fs::create_dir_all(repo_b.join(".snap")).unwrap();
        fs::write(repo_b.join(".snap/config.toml"), "[project]\nname = \"b\"\n").unwrap();
        set_active(&paths, &repo_b).unwrap();
        let got = resolve_repo(&repo_a, &paths).unwrap();
        assert_eq!(got, repo_a);
    }

    #[test]
    fn resolve_falls_back_to_active() {
        let (tmp, paths) = setup();
        let repo = tmp.path().join("proj");
        fs::create_dir_all(repo.join(".snap")).unwrap();
        fs::write(repo.join(".snap/config.toml"), "[project]\nname = \"p\"\n").unwrap();
        set_active(&paths, &repo).unwrap();
        let other = tmp.path().join("other");
        fs::create_dir_all(&other).unwrap();
        let got = resolve_repo(&other, &paths).unwrap();
        assert_eq!(got, repo);
    }

    #[test]
    fn resolve_errors_when_nothing_valid() {
        let (tmp, paths) = setup();
        let err = resolve_repo(tmp.path(), &paths).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("snap init"), "{msg}");
        assert!(msg.contains("snap use"), "{msg}");
    }

    #[test]
    fn upsert_is_idempotent_on_path() {
        let (_tmp, paths) = setup();
        let repo = paths.snap_home.join("never-mind");
        upsert_project(&paths, &repo, "n", Utc::now()).unwrap();
        upsert_project(&paths, &repo, "n", Utc::now()).unwrap();
        let reg = crate::state::ProjectRegistry::load(&paths.projects).unwrap();
        assert_eq!(reg.projects.len(), 1);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib project::`
Expected: FAIL — module missing

- [ ] **Step 3: Write minimal implementation**

`state.toml`:

```toml
active_project = "/abs/path"
```

`projects.toml`:

```toml
[[projects]]
path = "/abs/path"
name = "snap"
inited_at = "2026-08-18T14:20:15.897Z"
```

```rust
// StateError and ResolveError
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("{0}")]
    Io(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("No Snap project here. Run snap init in a repo, or snap use <path>.")]
    NoProject,
    #[error("{0}")]
    State(#[from] StateError),
}

#[derive(Debug, thiserror::Error)]
pub enum InitError {
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    State(#[from] StateError),
}
```

`GlobalState` / `ProjectRegistry` load missing files as empty. `set_active` writes `state.toml`. `upsert_project` replaces the entry with the same `path`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib project:: state::`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/state.rs src/project.rs src/lib.rs
git commit -m "feat: add project registry and repo resolve"
```

---

### Task 5: `snap init` and agent markdown

**Files:**
- Modify: `src/project.rs` — add `init_project`, `patch_agent_markdown`, `agent_markdown_block`
- Test: same file

**Interfaces:**
- Consumes: `Paths`, `set_active`, `upsert_project`, `find_git_root`
- Produces: `init_project(cwd, paths, now) -> Result<PathBuf, InitError>` (returns the repo root). Creates `.snap/config.toml`, `.snap/inbox/`, `.snap/.gitignore`. Registers the repo. Sets last-active. Patches markdown.

`.snap/.gitignore` contents (exact):

```
inbox/
last_read
```

`.snap/config.toml` contents:

```toml
[project]
name = "<directory name>"
```

`agent_markdown_block()` must return exactly:

```markdown
<!-- snap-agent-block:start -->
## Snap

This repo uses Snap for visual annotations.

- At the start of a UI task, run `snap check`.
- If `has_new` is true, run `snap latest` or `snap list`.
- Read the PNG at `image_path`. The image has the markup.
- Red circles/rectangles mark a problem.
- Arrows mark a move or a connection.
- Text labels are instructions.
- Numbered markers are an ordered list.
- Freehand marks an area.

After the user accepts the work, run `snap clear`.
<!-- snap-agent-block:end -->
```

Patch rules:
- If `AGENTS.md` exists, patch it.
- Else if `CLAUDE.md` exists, patch it.
- Else create `AGENTS.md`.
- If `AGENT_BLOCK_START` is already present, replace from that marker through `AGENT_BLOCK_END` (inclusive) with one fresh block. Do not append a second block.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn init_creates_layout_and_agents_md() {
    let (tmp, paths) = setup();
    let repo = tmp.path().join("app");
    fs::create_dir_all(repo.join(".git")).unwrap();
    let root = init_project(&repo, &paths, Utc::now()).unwrap();
    assert_eq!(root, repo);
    assert!(repo.join(".snap/config.toml").exists());
    assert!(repo.join(".snap/inbox").is_dir());
    let gi = fs::read_to_string(repo.join(".snap/.gitignore")).unwrap();
    assert!(gi.contains("inbox/"));
    assert!(gi.contains("last_read"));
    let md = fs::read_to_string(repo.join("AGENTS.md")).unwrap();
    assert!(md.contains(AGENT_BLOCK_START));
    assert!(md.contains("snap check"));
    let state = crate::state::GlobalState::load(&paths.state).unwrap();
    assert_eq!(state.active_project.as_deref(), Some(repo.as_path()));
}

#[test]
fn init_twice_does_not_duplicate_block() {
    let (tmp, paths) = setup();
    let repo = tmp.path().join("app");
    fs::create_dir_all(repo.join(".git")).unwrap();
    init_project(&repo, &paths, Utc::now()).unwrap();
    init_project(&repo, &paths, Utc::now()).unwrap();
    let md = fs::read_to_string(repo.join("AGENTS.md")).unwrap();
    assert_eq!(md.matches(AGENT_BLOCK_START).count(), 1);
}

#[test]
fn init_patches_existing_claude_md() {
    let (tmp, paths) = setup();
    let repo = tmp.path().join("app");
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::write(repo.join("CLAUDE.md"), "# Project\n").unwrap();
    init_project(&repo, &paths, Utc::now()).unwrap();
    assert!(!repo.join("AGENTS.md").exists());
    let md = fs::read_to_string(repo.join("CLAUDE.md")).unwrap();
    assert!(md.starts_with("# Project"));
    assert!(md.contains(AGENT_BLOCK_START));
}
```

Reuse `setup()` from Task 4 tests in the same module.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib project::tests::init_creates_layout_and_agents_md`
Expected: FAIL — `init_project` missing

- [ ] **Step 3: Write minimal implementation**

```rust
pub fn agent_markdown_block() -> String {
    format!(
        "{AGENT_BLOCK_START}\n## Snap\n\nThis repo uses Snap for visual annotations.\n\n\
- At the start of a UI task, run `snap check`.\n\
- If `has_new` is true, run `snap latest` or `snap list`.\n\
- Read the PNG at `image_path`. The image has the markup.\n\
- Red circles/rectangles mark a problem.\n\
- Arrows mark a move or a connection.\n\
- Text labels are instructions.\n\
- Numbered markers are an ordered list.\n\
- Freehand marks an area.\n\n\
After the user accepts the work, run `snap clear`.\n\
{AGENT_BLOCK_END}\n"
    )
}

pub fn patch_agent_markdown(repo: &Path) -> Result<PathBuf, InitError> {
    let agents = repo.join("AGENTS.md");
    let claude = repo.join("CLAUDE.md");
    let path = if agents.exists() {
        agents
    } else if claude.exists() {
        claude
    } else {
        agents
    };
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let block = agent_markdown_block();
    let next = if let Some(start) = existing.find(AGENT_BLOCK_START) {
        let end = existing[start..]
            .find(AGENT_BLOCK_END)
            .map(|i| start + i + AGENT_BLOCK_END.len())
            .unwrap_or(existing.len());
        let mut s = String::new();
        s.push_str(&existing[..start]);
        s.push_str(&block);
        let rest = existing[end..].trim_start_matches('\n');
        if !rest.is_empty() {
            s.push('\n');
            s.push_str(rest);
        }
        s
    } else if existing.is_empty() {
        block
    } else {
        format!("{}\n\n{block}", existing.trim_end())
    };
    fs::write(&path, next).map_err(|e| InitError::Io(e.to_string()))?;
    Ok(path)
}

pub fn init_project(cwd: &Path, paths: &Paths, now: DateTime<Utc>) -> Result<PathBuf, InitError> {
    let repo = find_git_root(cwd).unwrap_or_else(|| cwd.to_path_buf());
    let snap = repo.join(".snap");
    fs::create_dir_all(snap.join("inbox")).map_err(|e| InitError::Io(e.to_string()))?;
    let name = repo.file_name().unwrap_or_default().to_string_lossy();
    fs::write(
        snap.join("config.toml"),
        format!("[project]\nname = \"{name}\"\n"),
    )
    .map_err(|e| InitError::Io(e.to_string()))?;
    fs::write(snap.join(".gitignore"), "inbox/\nlast_read\n")
        .map_err(|e| InitError::Io(e.to_string()))?;
    upsert_project(paths, &repo, &name, now)?;
    set_active(paths, &repo)?;
    patch_agent_markdown(&repo)?;
    Ok(repo)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib project::`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/project.rs
git commit -m "feat: add snap init and agent markdown block"
```

---

### Task 6: Agent CLI

**Files:**
- Create: `src/cli.rs`
- Modify: `src/lib.rs` — add `pub mod cli;`
- Modify: `src/main.rs` — call `snap::cli::run()`
- Create: `tests/cli_agent.rs`

**Interfaces:**
- Consumes: `Inbox`, `resolve_repo`, `init_project`, `Paths::discover`
- Produces: `cli::run() -> ExitCode`. Subcommands: `Init`, `Use { path: Option<PathBuf> }`, `Projects`, `Check`, `Latest`, `List { last_n: Option<usize> }`, `Get { id: String }`, `Clear`. Also parse (but stub with exit 1 "daemon not implemented") `Daemon`, `Start`, `Stop`, `Status`, `Doctor`, `Capture` so clap accepts the full surface. Replace stubs in Tasks 8 and 12.

Exit codes: clap usage errors → 2. `InboxError::Empty` / `NotFound` / `ResolveError` → 1 and JSON `{"error":"<Display>"}` on stdout for agent commands.

`list` default `last_n` is 5.

`use` without path uses `find_git_root(cwd).unwrap_or(cwd)`.

`projects` prints one path per line, with `*` before the active path. Not JSON.

- [ ] **Step 1: Write the failing integration test**

`tests/cli_agent.rs`:

```rust
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
    assert!(out.status.success(), "stderr={}", String::from_utf8_lossy(&out.stderr));
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test cli_agent`
Expected: FAIL — `init` is not a clap command (binary prints `snap` and exits 0, or unknown command)

- [ ] **Step 3: Write minimal implementation**

`src/cli.rs` — clap `Parser` / `Subcommand` with the enum from Shared types command list. `run()`:

```rust
pub fn run() -> std::process::ExitCode {
    match run_inner() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{err:#}");
            std::process::ExitCode::from(1)
        }
    }
}
```

For agent commands, `print_json` + on `InboxError` print `{"error": e.to_string()}` and return exit 1.

`main.rs`:

```rust
fn main() {
    std::process::exit(u8::from(snap::cli::run()) as i32);
}
```

`ExitCode` converts via `.report()` on newer rustc; simpler:

```rust
fn main() {
    let _ = snap::cli::run();
}
```

and have `run()` call `std::process::exit` only at the end — prefer returning `ExitCode` from `main`:

```rust
fn main() -> std::process::ExitCode {
    snap::cli::run()
}
```

`use` writes last-active. If `send_request` does not exist yet, skip the daemon notify (Task 8 adds it). Do not fail `use`/`init` when the socket is missing.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test cli_agent && cargo test --lib`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/cli.rs src/main.rs src/lib.rs tests/cli_agent.rs
git commit -m "feat: add agent CLI and snap init command"
```

---

### Task 7: IPC codec and client

**Files:**
- Create: `src/ipc.rs`
- Modify: `src/lib.rs` — add `pub mod ipc;`

**Interfaces:**
- Consumes: nothing from inbox
- Produces: `Request`, `Response`, `Response::ok`, `Response::err`, `send_request`, `read_request(stream)`, `write_response(stream, &Response)`, `IpcError`

Protocol: one JSON object, one `\n`, then one JSON object. Then close.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::thread;

    #[test]
    fn request_tag_is_op() {
        let v = serde_json::to_value(Request::Ping).unwrap();
        assert_eq!(v, serde_json::json!({"op":"ping"}));
        let v = serde_json::to_value(Request::Use { path: "/tmp/r".into() }).unwrap();
        assert_eq!(v, serde_json::json!({"op":"use","path":"/tmp/r"}));
    }

    #[test]
    fn send_request_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let sock = tmp.path().join("s.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            let req: Request = serde_json::from_str(line.trim()).unwrap();
            assert_eq!(req, Request::Ping);
            let resp = Response::ok(serde_json::json!({"pid": 1}));
            writeln!(stream, "{}", serde_json::to_string(&resp).unwrap()).unwrap();
        });
        let resp = send_request(&sock, &Request::Ping).unwrap();
        assert!(resp.ok);
        assert_eq!(resp.data.unwrap()["pid"], 1);
        handle.join().unwrap();
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib ipc::`
Expected: FAIL — module missing

- [ ] **Step 3: Write minimal implementation**

```rust
#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("daemon is not running")]
    NotRunning,
    #[error("{0}")]
    Io(String),
    #[error("invalid ipc: {0}")]
    Codec(String),
}

pub fn send_request(sock: &Path, req: &Request) -> Result<Response, IpcError> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    let mut stream = UnixStream::connect(sock).map_err(|_| IpcError::NotRunning)?;
    let line = serde_json::to_string(req).map_err(|e| IpcError::Codec(e.to_string()))?;
    writeln!(stream, "{line}").map_err(|e| IpcError::Io(e.to_string()))?;
    let mut reply = String::new();
    BufReader::new(&stream)
        .read_line(&mut reply)
        .map_err(|e| IpcError::Io(e.to_string()))?;
    serde_json::from_str(reply.trim()).map_err(|e| IpcError::Codec(e.to_string()))
}
```

`Response::ok` sets `ok: true`. `Response::err` sets `ok: false` and `error`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib ipc::`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/ipc.rs src/lib.rs
git commit -m "feat: add daemon ipc codec"
```

---

### Task 8: Daemon socket, start, stop, status

**Files:**
- Create: `src/daemon.rs`
- Modify: `src/lib.rs` — add `pub mod daemon;`
- Modify: `src/cli.rs` — implement `Daemon`, `Start`, `Stop`, `Status`, `Capture` (capture sends `Request::Capture`; daemon may reply `{"ok":false,"error":"overlay not implemented"}` until Task 11)
- Modify: `src/project.rs` / `cli.rs` — `init` and `use` call `send_request(Request::Use { path })` and ignore `IpcError::NotRunning`
- Create: `tests/cli_daemon.rs`

**Interfaces:**
- Consumes: `Paths`, `Request`, `Response`, `send_request`, `GlobalState`
- Produces: `daemon::serve(paths: Paths) -> Result<(), anyhow::Error>` (blocking). `daemon::start_detached(paths: &Paths) -> Result<(), anyhow::Error>` waits ≤ 3_000 ms for ping. `daemon::pid_is_alive(pid: u32) -> bool`. Single instance: if ping succeeds, `snap daemon` prints that another instance is up and exits 0. `status` data keys: `pid`, `active_project`, `active_name`, `socket`.

In-memory last-active: `serve` loads `state.toml` at start. `Use` updates memory and writes `state.toml`. `Capture` in this task returns `Response::err("overlay not implemented")` and logs the event. Task 11 replaces that arm.

`start` if ping ok: print `snap daemon is already up` and exit 0.

`stop` sends `Shutdown`. If `NotRunning`, delete stale `sock` and `pid` if the pid is dead.

- [ ] **Step 1: Write the failing integration test**

```rust
// tests/cli_daemon.rs
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

    let init = snap().args(["init"]).env("HOME", &home).current_dir(&repo).output().unwrap();
    assert!(init.status.success(), "{}", String::from_utf8_lossy(&init.stderr));

    let start = snap().args(["start"]).env("HOME", &home).output().unwrap();
    assert!(start.status.success(), "{}", String::from_utf8_lossy(&start.stderr));
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test cli_daemon`
Expected: FAIL — `start` still stubbed

- [ ] **Step 3: Write minimal implementation**

`daemon::serve`:
1. `create_dir_all(snap_home)`
2. If `send_request(sock, Ping)` is ok, return `Ok(())` after logging "another instance"
3. Remove stale sock file
4. Bind `UnixListener`
5. Write pid file
6. Loop: accept, read one line, decode `Request`, handle, write `Response`, close
7. `Shutdown` breaks the loop, removes sock and pid, returns

`pid_is_alive`: `Command::new("kill").args(["-0", &pid.to_string()])` on Unix.

`start_detached`:

```rust
pub fn start_detached(paths: &Paths) -> Result<(), anyhow::Error> {
    if send_request(&paths.sock, &Request::Ping).is_ok() {
        println!("snap daemon is already up");
        return Ok(());
    }
    if paths.pid.exists() {
        if let Ok(s) = fs::read_to_string(&paths.pid) {
            if let Ok(pid) = s.trim().parse::<u32>() {
                if !pid_is_alive(pid) {
                    let _ = fs::remove_file(&paths.pid);
                    let _ = fs::remove_file(&paths.sock);
                }
            }
        }
    }
    let exe = std::env::current_exe()?;
    std::process::Command::new(exe)
        .arg("daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(3000);
    while std::time::Instant::now() < deadline {
        if send_request(&paths.sock, &Request::Ping).is_ok() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    anyhow::bail!("daemon did not answer ping within 3000 ms")
}
```

`status` human text:

```
pid: 123
active: /path/to/repo (name)
socket: /path/snap.sock
```

Load name from that repo's `.snap/config.toml` (`name` key) or the directory name.

Do not start GPUI in this task.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test cli_daemon -- --test-threads=1 && cargo test --lib`
Expected: PASS. If `cli_daemon` flakes on bind, keep `--test-threads=1`.

- [ ] **Step 5: Commit**

```bash
git add src/daemon.rs src/cli.rs src/lib.rs src/project.rs tests/cli_daemon.rs
git commit -m "feat: add snap daemon socket and lifecycle commands"
```

---

### Task 9: Draw session, crop, simplify

**Files:**
- Create: `src/draw.rs`
- Modify: `src/lib.rs` — add `pub mod draw;`

**Interfaces:**
- Consumes: `Point`, `Annotation`
- Produces: `Tool`, `Crop`, `CropDims`, `Session`, `map_crop_to_native`, `simplify_points`, `Session::undo`, `Session::clear`, `Session::add_marker`, `Session::to_annotations`

`map_crop_to_native` must match `app/src/export-scale.mjs`:

```javascript
const sx = nativeWidth / previewWidth;
const sy = nativeHeight / previewHeight;
x = clamp(round(crop.x * sx), 0, nativeWidth)
y = clamp(round(crop.y * sy), 0, nativeHeight)
w = clamp(round(crop.w * sx), 0, nativeWidth - x)
h = clamp(round(crop.h * sy), 0, nativeHeight - y)
```

`simplify_points`: keep first and last. Keep a mid point when hypot to last kept ≥ `tolerance`.

`Session::new()`: tool Circle, color `#FF3B30`, width 4.0, dim false, marker_counter 1.

`add_marker` pushes a marker and increments `marker_counter`.

`undo` pops. If the popped item is a marker, `marker_counter = marker_counter.saturating_sub(1).max(1)`.

`to_annotations` maps session items to `Annotation` (label `None` on shapes).

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn map_crop_scales_preview_to_native() {
    let mapped = map_crop_to_native(
        &Crop { x: 100.0, y: 50.0, w: 400.0, h: 200.0 },
        &CropDims {
            preview_width: 1920.0,
            preview_height: 1080.0,
            native_width: 3840.0,
            native_height: 2160.0,
        },
    )
    .unwrap();
    assert_eq!(mapped, Crop { x: 200.0, y: 100.0, w: 800.0, h: 400.0 });
}

#[test]
fn map_crop_clamps_to_native_bounds() {
    let mapped = map_crop_to_native(
        &Crop { x: -10.0, y: 1000.0, w: 2000.0, h: 300.0 },
        &CropDims {
            preview_width: 1000.0,
            preview_height: 1000.0,
            native_width: 2000.0,
            native_height: 2000.0,
        },
    )
    .unwrap();
    assert_eq!(mapped, Crop { x: 0.0, y: 2000.0, w: 2000.0, h: 0.0 });
}

#[test]
fn simplify_keeps_ends_and_drops_dense_mids() {
    let pts = vec![
        Point { x: 0.0, y: 0.0 },
        Point { x: 0.5, y: 0.0 },
        Point { x: 3.0, y: 0.0 },
    ];
    let out = simplify_points(&pts, 2.0);
    assert_eq!(out.first().unwrap().x, 0.0);
    assert_eq!(out.last().unwrap().x, 3.0);
    assert_eq!(out.len(), 2);
}

#[test]
fn undo_marker_decrements_counter() {
    let mut s = Session::new();
    s.add_marker(1.0, 2.0);
    s.add_marker(3.0, 4.0);
    assert_eq!(s.marker_counter, 3);
    s.undo();
    assert_eq!(s.marker_counter, 2);
    assert_eq!(s.to_annotations().len(), 1);
    s.clear();
    assert!(s.to_annotations().is_empty());
    assert_eq!(s.marker_counter, 1);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib draw::`
Expected: FAIL — module missing

- [ ] **Step 3: Write minimal implementation**

Port `mapCropToNative` with `f64` and `as i64` / `round`. Implement `SessionItem` as an enum that can hold the six live shapes (include `stroke_width: f32` on stroke tools). `to_annotations` drops stroke width (sidecar has no stroke field, matching the fork JSON). Keep stroke width on `SessionItem` for composite in Task 10.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib draw::`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/draw.rs src/lib.rs
git commit -m "feat: add draw session, crop map, and simplify"
```

---

### Task 10: Composite PNG and capture adapters

**Files:**
- Modify: `src/draw.rs` — add `composite_png`
- Create: `src/capture.rs`
- Create: `src/window.rs`
- Modify: `src/lib.rs`
- Create: `assets/LiberationSans-Bold.ttf` (download in the step)
- Modify: `Cargo.toml` — add `tiny-skia`, `fontdue`, `png`, `image` (`png` feature)

**Interfaces:**
- Consumes: `Annotation`, `Crop`, `Session`
- Produces: `composite_png(...)`, `ScreenCapture`, `CliCapture` (tries platform tools), `ActiveWindow`, `CliWindow`, `CaptureError`, `DrawError`

`composite_png`:
1. Decode `capture_png` with the `image` crate.
2. If `crop` is `Some`, apply `map_crop_to_native` using preview = decoded size if the caller already mapped — **caller passes native crop**. The function clips that rect from the decode.
3. Scale factor `sx = export_w / logical_w`, `sy = export_h / logical_h`.
4. Draw annotations with `tiny-skia` (stroke circle/rect/arrow/freehand, fill marker, text via fontdue + the bundled font).
5. Encode PNG8/RGBA8 with the `png` crate.

Arrow head: length `12 + width`, angle `± PI/6` (same as `app/src/main.js`).

Rect fill: color at 10% alpha (`0x1A`).

Text: black stroke then fill (approximate with a 3px darker draw offset if skia text stroke is hard). Marker: filled circle, white number, radius 16.

`CliCapture::capture_to`:
- macOS: `screencapture -i -s <path>`. Non-zero exit or empty file → `CaptureError::Cancelled` if the file is missing, else `CaptureError::Failed(msg)`.
- Linux: try `gnome-screenshot --file <path>`, then `grim <path>`, then `scrot --overwrite <path>`.

`CliWindow::read`:
- macOS: `osascript` as in `app/src-tauri/src/lib.rs` (`get_active_window_context_macos`).
- Linux Wayland (`WAYLAND_DISPLAY` set): `WindowSource::default()`.
- Linux X11: `xdotool` title/class/pid as in `lib.rs`.

Temp path: `std::env::temp_dir().join("snap-capture.png")`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn composite_writes_png_signature() {
    // 4x4 red PNG generated in the test
    let mut img = image::RgbaImage::new(4, 4);
    for p in img.pixels_mut() {
        *p = image::Rgba([255, 0, 0, 255]);
    }
    let mut buf = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
        .unwrap();
    let font = include_bytes!("../assets/LiberationSans-Bold.ttf");
    let out = composite_png(
        &buf,
        None,
        &[Annotation::Circle {
            center: [2.0, 2.0],
            radius: [1.0, 1.0],
            color: "#000000".into(),
            label: None,
        }],
        4.0,
        4.0,
        font,
    )
    .unwrap();
    assert_eq!(&out[..8], b"\x89PNG\r\n\x1a\n");
    assert!(out.len() > buf.len() / 2);
}

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
```

- [ ] **Step 2: Download the font, then run the test to see it fail**

```bash
mkdir -p assets
curl -fsSL -o assets/LiberationSans-Bold.ttf \
  https://github.com/liberationfonts/liberation-fonts/raw/2.1.5/liberation-fonts-ttf-2.1.5/LiberationSans-Bold.ttf
# If that URL 404s, use:
# https://cdn.jsdelivr.net/gh/liberationfonts/liberation-fonts@2.1.5/liberation-fonts-ttf-2.1.5/LiberationSans-Bold.ttf
file assets/LiberationSans-Bold.ttf   # must be a TTF
```

Add `assets/LiberationSans-Bold.ttf` to git.

Run: `cargo test --lib draw::tests::composite_writes_png_signature`
Expected: FAIL — `composite_png` missing

- [ ] **Step 3: Write minimal implementation**

Add to `Cargo.toml`:

```toml
fontdue = "0.9"
image = { version = "0.25", default-features = false, features = ["png"] }
png = "0.17"
tiny-skia = "0.11"
```

Implement `composite_png` and the two adapter modules. Keep OS commands behind the traits so tests never call `screencapture`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib draw:: capture:: window::`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/draw.rs src/capture.rs src/window.rs src/lib.rs Cargo.toml Cargo.lock assets/LiberationSans-Bold.ttf
git commit -m "feat: add png composite and capture adapters"
```

---

### Task 11: GPUI overlay, hotkey, save

**Files:**
- Create: `src/overlay.rs`
- Modify: `src/daemon.rs` — after the socket thread starts, enter `gpui_platform::application().run(...)`. Handle `Request::Capture` by sending a channel message that the GPUI app consumes to open the overlay.
- Modify: `src/cli.rs` — `Capture` requires a successful `send_request(Capture)` (`ok: true`)
- Modify: `Cargo.toml` — add gpui, gpui_platform, global-hotkey
- Modify: `src/lib.rs` — `pub mod overlay;`

**Interfaces:**
- Consumes: `Inbox::save`, `Session`, `composite_png`, `CliCapture`, `CliWindow`, `ProjectRegistry`, `set_active`
- Produces: `overlay::run_app(paths: Paths, ipc_rx: Receiver<IpcEvent>)`. `IpcEvent` is `{ Capture, Use(PathBuf), Shutdown }`.

Pin GPUI:

```bash
git ls-remote https://github.com/zed-industries/zed.git HEAD
```

Put that SHA in `Cargo.toml`:

```toml
gpui = { git = "https://github.com/zed-industries/zed", rev = "<sha>" }
gpui_platform = { git = "https://github.com/zed-industries/zed", rev = "<sha>", features = ["font-kit"] }
```

If `cargo check` fails, walk `git log --until` back one week at a time (max 6 weeks) until it compiles. Record the final SHA in `Cargo.toml`.

Do **not** add `gpui-component` unless the toolbar cannot be built with raw GPUI. Prefer raw GPUI `div` + `gpui::actions!` so we do not take the styled kit. Skip `gpui-base` if raw GPUI is enough for a dark toolbar.

Window: fullscreen, no decorations, always on top. Background is the capture image. Toolbar at top center: tools C/R/A/F/T/N, six colors, widths 2/4/6, dim, undo, clear, save, project chip.

Keys (GPUI actions): same table as the spec §9. `Enter` saves. `Escape` cancels the live stroke or closes without write.

macOS capture: `screencapture -i -s` **before** the window opens. Cancel → no window. Linux: capture full, then selection phase (min 10×10) before the toolbar is active.

Save:
1. `composite_png` with bundled font `include_bytes!("../assets/LiberationSans-Bold.ttf")`
2. Build `Sidecar` with `WindowSource` from the pre-capture read, `resolution` from the capture image size, `timestamp` RFC3339 millis Z
3. `Inbox::new(active.join(".snap")).save(...)`
4. Close the overlay window. Daemon stays up.

Project chip: show `ProjectEntry.name`. Click cycles `ProjectRegistry.projects` whose `path` still exists, then `set_active`. Save uses the path at Enter time.

Hotkey: `global-hotkey` crate, `Control+Shift+S`. On fire, run the same path as `IpcEvent::Capture`. If last-active has no `.snap/config.toml`, log and do not open a window.

macOS accessory: set the activation policy to accessory / `LSUIElement` equivalent that GPUI exposes. If the API name on the pinned crate differs, use that crate's accessory policy. Do not show a Dock bounce.

`snap daemon` now: spawn socket thread that forwards decoded requests to `std::sync::mpsc::Sender<IpcEvent>`, then `overlay::run_app`. `Shutdown` sends the event; the GPUI app quits.

- [ ] **Step 1: Write the failing unit test (no GPU)**

The overlay is not GPU-tested. Extend `Session` tests if needed, and add a save-path unit test in `daemon` or `overlay` that does not open a window:

```rust
#[test]
fn save_session_writes_pair_to_active_inbox() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(repo.join(".snap/inbox")).unwrap();
    let mut img = image::RgbaImage::new(4, 4);
    for p in img.pixels_mut() {
        *p = image::Rgba([255, 0, 0, 255]);
    }
    let mut buf = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
        .unwrap();
    let mut session = Session::new();
    session.add_marker(2.0, 2.0);
    let path = save_session(
        &repo,
        &buf,
        None,
        WindowSource::default(),
        &session,
        4.0,
        4.0,
        Utc::now(),
    )
    .unwrap();
    assert!(path.exists());
    let rec = Inbox::new(repo.join(".snap")).latest(false).unwrap();
    assert!(matches!(rec.annotations[0], Annotation::Marker { number: 1, .. }));
}
```

Put `pub fn save_session` in `src/overlay.rs`. The GPUI Enter handler only calls that function.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib overlay::tests::save_session_writes_pair_to_active_inbox`
Expected: FAIL — `overlay` / `save_session` missing

- [ ] **Step 3: Write `save_session` and the GPUI app**

```rust
pub fn save_session(
    repo: &Path,
    capture_png: &[u8],
    crop: Option<&Crop>,
    mut source: WindowSource,
    session: &Session,
    logical_w: f64,
    logical_h: f64,
    now: DateTime<Utc>,
) -> Result<PathBuf, anyhow::Error> {
    let font = include_bytes!("../assets/LiberationSans-Bold.ttf");
    let dynimg = image::load_from_memory(capture_png)?;
    source.resolution = [dynimg.width(), dynimg.height()];
    let native_crop = crop.and_then(|c| {
        map_crop_to_native(
            c,
            &CropDims {
                preview_width: logical_w,
                preview_height: logical_h,
                native_width: dynimg.width() as f64,
                native_height: dynimg.height() as f64,
            },
        )
    });
    let png = composite_png(
        capture_png,
        native_crop.as_ref(),
        &session.to_annotations(),
        logical_w,
        logical_h,
        font,
    )?;
    let mut sidecar = Sidecar {
        timestamp: now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        source,
        annotations: session.to_annotations(),
        image_filename: String::new(),
    };
    Ok(Inbox::new(repo.join(".snap")).save(&mut sidecar, &png, now)?)
}
```

Wire `Request::Capture` to this session in the GPUI callback. Implement toolbar + keys. If GPUI window flags on the pinned crate use different field names, map: undecorated, always on top, fullscreen.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib && cargo test --test cli_agent && cargo test --test cli_daemon -- --test-threads=1`
Expected: PASS. Then manual on the host (not a CI gate): `cargo run -- start`, `cargo run -- capture`, draw, Enter, `cargo run -- latest`.

- [ ] **Step 5: Commit**

```bash
git add src/overlay.rs src/daemon.rs src/cli.rs src/lib.rs Cargo.toml Cargo.lock
git commit -m "feat: add gpui overlay, hotkey, and save"
```

---

### Task 12: Doctor

**Files:**
- Create: `src/doctor.rs`
- Modify: `src/lib.rs`, `src/cli.rs`

**Interfaces:**
- Consumes: `Paths`, `send_request`, `GlobalState`
- Produces: `doctor::run(paths: &Paths) -> ExitCode`. Prints one check per line. Exit 1 if any check fails.

Checks:
1. `snap_home` exists or can be created
2. Ping socket → `daemon: up` or `daemon: down`
3. `active_project` set and `<path>/.snap/config.toml` exists
4. Capture tool: macOS `screencapture` on `PATH`; Linux at least one of `gnome-screenshot`, `grim`, `scrot`
5. macOS Screen Recording: `CGPreflightScreenCaptureAccess`; if false, print `Screen Recording permission is missing. Enable it in System Settings > Privacy & Security > Screen Recording.`
6. If `home.join(".snap/inbox")` exists and has `snap-*.json`, print `note: old global inbox at ~/.snap/inbox is not used. Per-repo inbox is <repo>/.snap/inbox/`

```rust
#[cfg(target_os = "macos")]
fn screen_recording_ok() -> bool {
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
    }
    unsafe { CGPreflightScreenCaptureAccess() }
}
```

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn doctor_reports_missing_active_project() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::from_snap_home(tmp.path().join(".snap"));
    std::fs::create_dir_all(&paths.snap_home).unwrap();
    let report = crate::doctor::report(&paths);
    assert!(report.iter().any(|l| l.contains("active") || l.contains("snap init")));
}
```

Put `pub fn report(paths: &Paths) -> Vec<String>` behind `run`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib doctor::`
Expected: FAIL — module missing

- [ ] **Step 3: Write minimal implementation**

Human-readable lines only. No JSON.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib doctor::`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/doctor.rs src/cli.rs src/lib.rs
git commit -m "feat: add snap doctor"
```

---

### Task 13: Cutover docs and delete the old tree

**Files:**
- Modify: `snap.plist` — `ProgramArguments` is the installed binary plus `daemon` (not `open -a /Applications/snap.app`)
- Modify: `snap.service` — `ExecStart` is `%h/.local/bin/snap daemon` (or `/usr/local/bin/snap daemon`). Remove `WEBKIT_DISABLE_COMPOSITING_MODE`.
- Modify: `Makefile` — `build` is `cargo build --release`. `install` copies the binary to `~/.local/bin/snap` and writes launchd/systemd to run `snap daemon`. Remove npm/tauri/uv/mcp targets.
- Modify: `README.md`, `SETUP.md`, `CLAUDE.md` — CLI + daemon + `snap init`. No MCP, no Tauri, no Node, no Python.
- Delete: `app/`, `mcp-server/`, `setup-mcp.sh`, `snap-trigger.sh` (v1 has no Wayland trigger script)

Do this only after Task 11 save works on the implementation host: `snap capture` writes a pair and `snap latest` reads it.

**Interfaces:**
- Consumes: the new CLI
- Produces: docs that match the spec command surface

- [ ] **Step 1: Confirm the overlay save path works**

Run:

```bash
cargo build --release
./target/release/snap init
./target/release/snap start
./target/release/snap capture
# annotate, Enter
./target/release/snap latest
```

Expected: JSON with `image_path` under this repo's `.snap/inbox/`. Do not delete `app/` if this fails.

- [ ] **Step 2: Rewrite Makefile**

```makefile
.PHONY: build install start stop restart logs status clean

build:
	cargo build --release

install: build
	mkdir -p $(HOME)/.local/bin
	cp target/release/snap $(HOME)/.local/bin/snap
ifeq ($(shell uname -s),Darwin)
	mkdir -p $(HOME)/Library/LaunchAgents
	sed 's|%SNAP_BIN%|$(HOME)/.local/bin/snap|g; s|%HOME_PATH%|$(HOME)|g' snap.plist \
	  > $(HOME)/Library/LaunchAgents/com.adjective.snap.plist
	launchctl unload $(HOME)/Library/LaunchAgents/com.adjective.snap.plist 2>/dev/null || true
	launchctl load -w $(HOME)/Library/LaunchAgents/com.adjective.snap.plist
else
	mkdir -p $(HOME)/.config/systemd/user
	sed 's|%SNAP_BIN%|$(HOME)/.local/bin/snap|g' snap.service > $(HOME)/.config/systemd/user/snap.service
	systemctl --user daemon-reload
	systemctl --user enable --now snap.service
endif

start:
	$(HOME)/.local/bin/snap start

stop:
	$(HOME)/.local/bin/snap stop

status:
	$(HOME)/.local/bin/snap status

logs:
	tail -f $(HOME)/.snap/snap.log

clean:
	cargo clean
```

`snap.plist` `ProgramArguments`: `%SNAP_BIN%`, `daemon`.

`snap.service`:

```
[Service]
Type=simple
ExecStart=%SNAP_BIN% daemon
Restart=on-failure
RestartSec=3
```

- [ ] **Step 3: Rewrite README, SETUP, CLAUDE.md**

README must document:
- `snap init`, `snap start`, `Ctrl+Shift+S`, `snap latest`, `snap clear`
- Per-repo inbox
- Last-active project
- No MCP

CLAUDE.md must drop Tauri/MCP build notes and say:

```
Always load the rust skill at the start of each session when working in this project.
```

Plus the four roles and `cargo test` / `cargo build --release`. Never `npx tauri build`.

SETUP.md: Rust toolchain only. macOS Screen Recording + Accessibility. `make install`.

- [ ] **Step 4: Delete the old tree**

```bash
git rm -r app mcp-server setup-mcp.sh snap-trigger.sh
```

Keep `LICENSE`. Keep `docs/`.

- [ ] **Step 5: Run the full test suite**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: PASS. No references to `mcp-server` or `tauri` in the remaining tree except historical notes in `docs/superpowers/`.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: cut over to snap cli and remove tauri mcp tree"
```

---

## Spec coverage

| Spec item | Task |
|-----------|------|
| One binary, four roles | 1, 6, 8, 11 |
| Agent commands on disk | 3, 6 |
| Per-repo inbox + schema | 2, 3 |
| last_read rules | 3 |
| `snap init` + markdown | 5 |
| Resolve cwd then last-active | 4 |
| Socket protocol | 7, 8 |
| start/stop/status/single-instance | 8 |
| 3s start timeout, stale pid | 8 |
| Crop map + tools + undo | 9, 11 |
| tiny-skia composite | 10 |
| macOS/Linux capture + window | 10, 11 |
| GPUI overlay + hotkey + chip | 11 |
| Log rotate 1 MB | 1 |
| doctor + old inbox note | 12 |
| Delete app/ + mcp-server | 13 |
| plist/service/Makefile/docs | 13 |
| No MCP, no Windows overlay | (non-goal; not tasked) |

## Type check

`Inbox::save`, `Annotation`, `Request::Use { path }`, `Paths::from_snap_home`, `map_crop_to_native`, `Session::to_annotations`, `save_session` keep the same names from Task 2 through Task 11.
