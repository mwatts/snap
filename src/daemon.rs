use crate::ipc::{Request, Response, read_request, send_request, write_response};
use crate::logutil::log_event;
use crate::overlay::{self, IpcEvent};
use crate::paths::Paths;
use crate::state::GlobalState;
use std::fs;
use std::io::BufReader;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;

pub fn pid_is_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

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

pub fn serve(paths: Paths) -> Result<(), anyhow::Error> {
    fs::create_dir_all(&paths.snap_home)?;
    if send_request(&paths.sock, &Request::Ping).is_ok() {
        println!("another instance is up");
        log_event(&paths, "another instance is up");
        return Ok(());
    }
    if paths.sock.exists() {
        let _ = fs::remove_file(&paths.sock);
    }
    let listener = UnixListener::bind(&paths.sock)?;
    fs::write(&paths.pid, format!("{}\n", std::process::id()))?;
    log_event(&paths, "daemon listening");

    let (tx, rx) = mpsc::channel();
    let paths_sock = paths.clone();
    std::thread::spawn(move || socket_loop(paths_sock, listener, tx));
    overlay::run_app(paths.clone(), rx);

    let _ = fs::remove_file(&paths.sock);
    let _ = fs::remove_file(&paths.pid);
    log_event(&paths, "daemon stopped");
    Ok(())
}

fn socket_loop(paths: Paths, listener: UnixListener, tx: mpsc::Sender<IpcEvent>) {
    let mut active_project = GlobalState::load(&paths.state)
        .unwrap_or_default()
        .active_project;

    loop {
        let (stream, _) = match listener.accept() {
            Ok(pair) => pair,
            Err(e) => {
                log_event(&paths, &format!("accept: {e}"));
                continue;
            }
        };
        let mut reader = BufReader::new(stream);
        let req = match read_request(&mut reader) {
            Ok(req) => req,
            Err(e) => {
                let _ = write_response(reader.get_mut(), &Response::err(e.to_string()));
                continue;
            }
        };
        let shutdown = matches!(req, Request::Shutdown);
        let resp = handle(&paths, &mut active_project, &tx, req);
        let _ = write_response(reader.get_mut(), &resp);
        if shutdown {
            break;
        }
    }

    let _ = fs::remove_file(&paths.sock);
    let _ = fs::remove_file(&paths.pid);
}

fn handle(
    paths: &Paths,
    active: &mut Option<PathBuf>,
    tx: &mpsc::Sender<IpcEvent>,
    req: Request,
) -> Response {
    match req {
        Request::Ping => Response::ok(serde_json::json!({})),
        Request::Capture => {
            if !active
                .as_ref()
                .is_some_and(|repo| repo.join(".snap").join("config.toml").is_file())
            {
                log_event(paths, "capture: last-active has no .snap/config.toml");
                return Response::err("no active project");
            }
            if let Err(e) = tx.send(IpcEvent::Capture) {
                return Response::err(e.to_string());
            }
            log_event(paths, "capture");
            Response::ok(serde_json::json!({}))
        }
        Request::Use { path } => {
            let repo = PathBuf::from(path);
            *active = Some(repo.clone());
            if let Err(e) = (GlobalState {
                active_project: Some(repo.clone()),
            })
            .save(&paths.state)
            {
                return Response::err(e.to_string());
            }
            let _ = tx.send(IpcEvent::Use(repo.clone()));
            log_event(paths, &format!("use {}", repo.display()));
            Response::ok(serde_json::json!({}))
        }
        Request::Status => {
            let active_name = active.as_ref().map(|p| project_name(p));
            Response::ok(serde_json::json!({
                "pid": std::process::id(),
                "active_project": active,
                "active_name": active_name,
                "socket": paths.sock,
            }))
        }
        Request::Shutdown => {
            let _ = tx.send(IpcEvent::Shutdown);
            Response::ok(serde_json::json!({}))
        }
    }
}

pub(crate) fn project_name(repo: &Path) -> String {
    let cfg = repo.join(".snap").join("config.toml");
    if let Ok(text) = fs::read_to_string(cfg) {
        if let Ok(val) = text.parse::<toml::Value>() {
            if let Some(name) = val
                .get("project")
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
            {
                return name.to_string();
            }
            if let Some(name) = val.get("name").and_then(|n| n.as_str()) {
                return name.to_string();
            }
        }
    }
    repo.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| repo.display().to_string())
}
