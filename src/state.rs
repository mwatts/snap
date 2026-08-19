use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("{0}")]
    Io(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalState {
    pub active_project: Option<PathBuf>,
}

impl GlobalState {
    pub fn load(path: &Path) -> Result<Self, StateError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(path).map_err(|e| StateError::Io(e.to_string()))?;
        if text.trim().is_empty() {
            return Ok(Self::default());
        }
        toml::from_str(&text).map_err(|e| StateError::Io(e.to_string()))
    }

    pub fn save(&self, path: &Path) -> Result<(), StateError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| StateError::Io(e.to_string()))?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| StateError::Io(e.to_string()))?;
        fs::write(path, text).map_err(|e| StateError::Io(e.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectEntry {
    pub path: PathBuf,
    pub name: String,
    pub inited_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRegistry {
    pub projects: Vec<ProjectEntry>,
}

impl ProjectRegistry {
    pub fn load(path: &Path) -> Result<Self, StateError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(path).map_err(|e| StateError::Io(e.to_string()))?;
        if text.trim().is_empty() {
            return Ok(Self::default());
        }
        toml::from_str(&text).map_err(|e| StateError::Io(e.to_string()))
    }

    pub fn save(&self, path: &Path) -> Result<(), StateError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| StateError::Io(e.to_string()))?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| StateError::Io(e.to_string()))?;
        fs::write(path, text).map_err(|e| StateError::Io(e.to_string()))
    }
}
