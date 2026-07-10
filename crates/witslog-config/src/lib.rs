use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub db_scope: DbScope,
    pub db_path: Option<PathBuf>,
    #[serde(default)]
    pub retention: RetentionPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DbScope {
    Project,
    Global,
}

impl Default for DbScope {
    fn default() -> Self {
        DbScope::Project
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionPolicy {
    pub max_age_days: Option<u32>,
    pub max_rows: Option<u32>,
    pub max_bytes: Option<u64>,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        RetentionPolicy {
            max_age_days: Some(90),
            max_rows: None,
            max_bytes: None,
        }
    }
}

impl Config {
    pub fn default_project() -> Self {
        Config {
            db_scope: DbScope::Project,
            db_path: None,
            retention: RetentionPolicy::default(),
        }
    }

    pub fn resolve_db_path(&self, cwd: &Path) -> PathBuf {
        if let Some(ref path) = self.db_path {
            return path.clone();
        }

        match self.db_scope {
            DbScope::Project => resolve_project_db(cwd),
            DbScope::Global => resolve_global_db(),
        }
    }
}

pub fn resolve_project_db(cwd: &Path) -> PathBuf {
    let mut current = cwd.to_path_buf();

    loop {
        let witslog_dir = current.join(".witslog");
        if witslog_dir.exists() {
            return witslog_dir.join("witslog.db");
        }

        if !current.pop() {
            break;
        }
    }

    cwd.join(".witslog").join("witslog.db")
}

#[cfg(target_os = "windows")]
pub fn resolve_global_db() -> PathBuf {
    let appdata = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(appdata).join("witslog").join("global.db")
}

#[cfg(not(target_os = "windows"))]
pub fn resolve_global_db() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    #[cfg(target_os = "macos")]
    {
        PathBuf::from(home).join("Library/Application Support/witslog/global.db")
    }
    #[cfg(not(target_os = "macos"))]
    {
        let xdg_data = std::env::var("XDG_DATA_HOME")
            .unwrap_or_else(|_| format!("{}/.local/share", home));
        PathBuf::from(xdg_data).join("witslog/global.db")
    }
}
