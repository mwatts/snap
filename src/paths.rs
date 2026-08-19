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
        let home = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
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
