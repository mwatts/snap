use crate::inbox::{Inbox, InboxError};
use crate::paths::Paths;
use crate::project::{find_git_root, init_project, resolve_repo, set_active};
use crate::state::{GlobalState, ProjectRegistry};
use clap::{Parser, Subcommand};
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
        Commands::Daemon
        | Commands::Start
        | Commands::Stop
        | Commands::Status
        | Commands::Doctor
        | Commands::Capture => Ok(daemon_not_implemented()),
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

fn daemon_not_implemented() -> ExitCode {
    eprintln!("daemon not implemented");
    ExitCode::from(1)
}
