use crate::paths::Paths;
use crate::state::{GlobalState, ProjectEntry, ProjectRegistry, StateError};
use chrono::{DateTime, SecondsFormat, Utc};
use std::fs;
use std::path::{Path, PathBuf};

pub const AGENT_BLOCK_START: &str = "<!-- snap-agent-block:start -->";
pub const AGENT_BLOCK_END: &str = "<!-- snap-agent-block:end -->";

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

pub fn find_git_root(start: &Path) -> Option<PathBuf> {
    let mut cur = start.to_path_buf();
    loop {
        if cur.join(".git").exists() {
            return Some(cur);
        }
        if !cur.pop() {
            return None;
        }
    }
}

fn has_snap_config(dir: &Path) -> bool {
    dir.join(".snap").join("config.toml").is_file()
}

pub fn resolve_repo(cwd: &Path, paths: &Paths) -> Result<PathBuf, ResolveError> {
    let mut cur = cwd.to_path_buf();
    loop {
        if has_snap_config(&cur) {
            return Ok(cur);
        }
        if !cur.pop() {
            break;
        }
    }

    let state = GlobalState::load(&paths.state)?;
    if let Some(active) = state.active_project {
        if has_snap_config(&active) {
            return Ok(active);
        }
    }

    Err(ResolveError::NoProject)
}

pub fn set_active(paths: &Paths, repo: &Path) -> Result<(), StateError> {
    let state = GlobalState {
        active_project: Some(repo.to_path_buf()),
    };
    state.save(&paths.state)
}

pub fn upsert_project(
    paths: &Paths,
    repo: &Path,
    name: &str,
    now: DateTime<Utc>,
) -> Result<(), StateError> {
    let mut reg = ProjectRegistry::load(&paths.projects)?;
    let entry = ProjectEntry {
        path: repo.to_path_buf(),
        name: name.to_string(),
        inited_at: now.to_rfc3339_opts(SecondsFormat::Millis, true),
    };
    if let Some(existing) = reg.projects.iter_mut().find(|p| p.path == repo) {
        *existing = entry;
    } else {
        reg.projects.push(entry);
    }
    reg.save(&paths.projects)
}

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
}
