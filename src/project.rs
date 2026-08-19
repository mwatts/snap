use crate::paths::Paths;
use crate::state::{GlobalState, ProjectEntry, ProjectRegistry, StateError};
use chrono::{DateTime, SecondsFormat, Utc};
use std::path::{Path, PathBuf};

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
