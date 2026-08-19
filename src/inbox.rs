use crate::schema::{AnnotationRecord, CheckResult, Sidecar};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub struct Inbox {
    pub snap_dir: PathBuf,
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

impl Inbox {
    pub fn new(snap_dir: PathBuf) -> Self {
        Self { snap_dir }
    }

    fn inbox_dir(&self) -> PathBuf {
        self.snap_dir.join("inbox")
    }

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
        let sidecar: Sidecar =
            serde_json::from_str(&text).map_err(|e| InboxError::Corrupt(e.to_string()))?;
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
        let Some(p) = files.first() else {
            return Err(InboxError::Empty);
        };
        let rec = self.load_path(p)?;
        if mark_read {
            self.mark_read()?;
        }
        Ok(rec)
    }

    pub fn list(&self, last_n: usize, mark_read: bool) -> Result<Vec<AnnotationRecord>, InboxError> {
        let files = self.json_files_newest_first()?;
        let recs = files
            .iter()
            .take(last_n)
            .map(|p| self.load_path(p))
            .collect::<Result<Vec<_>, _>>()?;
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
        Ok(ClearResult {
            deleted_json,
            deleted_png,
            errors,
        })
    }

    pub fn save(
        &self,
        sidecar: &mut Sidecar,
        png: &[u8],
        now: DateTime<Utc>,
    ) -> Result<PathBuf, InboxError> {
        let dir = self.inbox_dir();
        fs::create_dir_all(&dir).map_err(|e| InboxError::Io(e.to_string()))?;
        let stem = now
            .format("%Y%m%d-%H%M%S%.3f")
            .to_string()
            .replace('.', "-");
        let stem = format!("snap-{stem}");
        let png_name = format!("{stem}.png");
        let png_path = dir.join(&png_name);
        let json_path = dir.join(format!("{stem}.json"));
        fs::write(&png_path, png).map_err(|e| InboxError::Io(e.to_string()))?;
        sidecar.image_filename = png_name;
        let text =
            serde_json::to_string_pretty(sidecar).map_err(|e| InboxError::Corrupt(e.to_string()))?;
        fs::write(&json_path, text).map_err(|e| InboxError::Io(e.to_string()))?;
        Ok(png_path)
    }
}

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
        fs::write(
            &json,
            serde_json::to_string(&side("snap-orphan.png")).unwrap(),
        )
        .unwrap();
        let rec = inbox.get("snap-orphan").unwrap();
        assert_eq!(rec.image_path, None);
        assert_eq!(rec.warning.as_deref(), Some("image file missing"));
    }
}
