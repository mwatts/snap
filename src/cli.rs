use crate::daemon;
use crate::inbox::{Inbox, InboxError};
use crate::ipc::{send_request, IpcError, Request, Response};
use crate::paths::Paths;
use crate::project::{find_git_root, init_project, resolve_repo, set_active};
use crate::state::{GlobalState, ProjectRegistry};
use clap::{Parser, Subcommand};
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Debug, Parser)]
#[command(name = "snap")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Init,
    Use { path: Option<PathBuf> },
    Projects,
    Check,
    Latest,
    List { last_n: Option<usize> },
    Get { id: String },
    Clear,
    Daemon,
    Start,
    Stop,
    Status,
    Doctor,
    Capture,
}

pub fn run() -> ExitCode {
    match run_inner() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{err:#}");
            ExitCode::from(1)
        }
    }
}

fn run_inner() -> anyhow::Result<ExitCode> {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            err.print().ok();
            let code = u8::try_from(err.exit_code()).unwrap_or(2);
            return Ok(ExitCode::from(code));
        }
    };

    match cli.command {
        Commands::Init => cmd_init(),
        Commands::Use { path } => cmd_use(path),
        Commands::Projects => cmd_projects(),
        Commands::Check => agent(|inbox| inbox.check()),
        Commands::Latest => agent(|inbox| inbox.latest(true)),
        Commands::List { last_n } => agent(|inbox| inbox.list(last_n.unwrap_or(5), true)),
        Commands::Get { id } => agent(|inbox| inbox.get(&id)),
        Commands::Clear => agent(|inbox| inbox.clear()),
        Commands::Daemon => cmd_daemon(),
        Commands::Start => cmd_start(),
        Commands::Stop => cmd_stop(),
        Commands::Status => cmd_status(),
        Commands::Capture => cmd_capture(),
        Commands::Doctor => Ok(daemon_not_implemented()),
    }
}

fn cmd_init() -> anyhow::Result<ExitCode> {
    let paths = Paths::discover()?;
    let cwd = std::env::current_dir()?;
    let repo = init_project(&cwd, &paths, chrono::Utc::now())?;
    println!("initialized {}", repo.display());
    Ok(ExitCode::from(0))
}

fn cmd_use(path: Option<PathBuf>) -> anyhow::Result<ExitCode> {
    let paths = Paths::discover()?;
    let cwd = std::env::current_dir()?;
    let repo = match path {
        Some(p) if p.is_absolute() => p,
        Some(p) => cwd.join(p),
        None => find_git_root(&cwd).unwrap_or(cwd),
    };
    set_active(&paths, &repo)?;
    println!("using {}", repo.display());
    Ok(ExitCode::from(0))
}

fn cmd_projects() -> anyhow::Result<ExitCode> {
    let paths = Paths::discover()?;
    let reg = ProjectRegistry::load(&paths.projects)?;
    let state = GlobalState::load(&paths.state)?;
    for entry in &reg.projects {
        if state.active_project.as_ref() == Some(&entry.path) {
            println!("*{}", entry.path.display());
        } else {
            println!("{}", entry.path.display());
        }
    }
    Ok(ExitCode::from(0))
}

fn agent<T: serde::Serialize>(
    f: impl FnOnce(&Inbox) -> Result<T, InboxError>,
) -> anyhow::Result<ExitCode> {
    let paths = Paths::discover()?;
    let cwd = std::env::current_dir()?;
    let repo = match resolve_repo(&cwd, &paths) {
        Ok(repo) => repo,
        Err(err) => return Ok(agent_error(err)),
    };
    let inbox = Inbox::new(repo.join(".snap"));
    match f(&inbox) {
        Ok(value) => {
            println!("{}", serde_json::to_string(&value)?);
            Ok(ExitCode::from(0))
        }
        Err(err) => Ok(agent_error(err)),
    }
}

fn agent_error(err: impl std::fmt::Display) -> ExitCode {
    let msg = err.to_string();
    println!("{}", serde_json::json!({ "error": msg }));
    eprintln!("{msg}");
    ExitCode::from(1)
}

fn cmd_daemon() -> anyhow::Result<ExitCode> {
    let paths = Paths::discover()?;
    daemon::serve(paths)?;
    Ok(ExitCode::from(0))
}

fn cmd_start() -> anyhow::Result<ExitCode> {
    let paths = Paths::discover()?;
    daemon::start_detached(&paths)?;
    Ok(ExitCode::from(0))
}

fn cmd_stop() -> anyhow::Result<ExitCode> {
    let paths = Paths::discover()?;
    match send_request(&paths.sock, &Request::Shutdown) {
        Ok(_) => Ok(ExitCode::from(0)),
        Err(IpcError::NotRunning) => {
            let dead = match fs::read_to_string(&paths.pid) {
                Ok(s) => s
                    .trim()
                    .parse::<u32>()
                    .map(|pid| !daemon::pid_is_alive(pid))
                    .unwrap_or(true),
                Err(_) => true,
            };
            if dead {
                let _ = fs::remove_file(&paths.sock);
                let _ = fs::remove_file(&paths.pid);
            }
            Ok(ExitCode::from(0))
        }
        Err(e) => Err(e.into()),
    }
}

fn cmd_status() -> anyhow::Result<ExitCode> {
    let paths = Paths::discover()?;
    match send_request(&paths.sock, &Request::Status) {
        Ok(resp) if resp.ok => {
            print_status(&resp);
            Ok(ExitCode::from(0))
        }
        Ok(resp) => {
            eprintln!("{}", resp.error.as_deref().unwrap_or("status failed"));
            Ok(ExitCode::from(1))
        }
        Err(IpcError::NotRunning) => {
            eprintln!("daemon is not running");
            Ok(ExitCode::from(1))
        }
        Err(e) => Err(e.into()),
    }
}

fn cmd_capture() -> anyhow::Result<ExitCode> {
    let paths = Paths::discover()?;
    match send_request(&paths.sock, &Request::Capture) {
        Ok(resp) => {
            println!("{}", serde_json::to_string(&resp)?);
            if resp.ok {
                Ok(ExitCode::from(0))
            } else {
                if let Some(err) = &resp.error {
                    eprintln!("{err}");
                }
                Ok(ExitCode::from(1))
            }
        }
        Err(IpcError::NotRunning) => {
            eprintln!("daemon is not running");
            Ok(ExitCode::from(1))
        }
        Err(e) => Err(e.into()),
    }
}

fn print_status(resp: &Response) {
    let data = resp.data.clone().unwrap_or(serde_json::json!({}));
    let pid = data
        .get("pid")
        .and_then(|v| v.as_u64())
        .map(|n| n.to_string())
        .unwrap_or_else(|| "-".into());
    let socket = data.get("socket").and_then(|v| v.as_str()).unwrap_or("-");
    let active = match data.get("active_project").and_then(|v| v.as_str()) {
        Some(path) => {
            let name = data
                .get("active_name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if name.is_empty() {
                path.to_string()
            } else {
                format!("{path} ({name})")
            }
        }
        None => "none".into(),
    };
    println!("pid: {pid}");
    println!("active: {active}");
    println!("socket: {socket}");
}

fn daemon_not_implemented() -> ExitCode {
    eprintln!("daemon not implemented");
    ExitCode::from(1)
}
