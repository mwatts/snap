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
        f.write_all(&vec![b'x'; (LOG_ROTATE_BYTES + 1) as usize]).unwrap();
        drop(f);
        rotate_if_needed(&paths).unwrap();
        assert!(paths.snap_home.join("snap.log.old").exists());
        assert!(!paths.log.exists() || fs::metadata(&paths.log).unwrap().len() == 0);
    }
}
