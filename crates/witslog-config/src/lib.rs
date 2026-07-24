use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub db_scope: DbScope,
    pub db_path: Option<PathBuf>,
    pub retention: RetentionPolicy,
    pub enrich: EnrichSection,
    pub redact: RedactSection,
    pub buffer: BufferSection,
    pub taxonomy: TaxonomySection,
    pub notify: NotifySection,
    pub crypto: CryptoSection,
}

impl Default for Config {
    fn default() -> Self {
        Config::default_project()
    }
}

/// Mirrors `witslog_core::EnrichConfig`. Kept as a plain data struct here (this
/// crate stays leaf/dependency-free of witslog-core) — callers build the
/// witslog-core type field-by-field from this.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EnrichSection {
    pub hostname: bool,
    pub pid: bool,
    pub cwd: bool,
    pub argv: bool,
    pub git_commit: bool,
    pub env_allowlist: Vec<String>,
}

impl Default for EnrichSection {
    fn default() -> Self {
        EnrichSection {
            hostname: true,
            pid: true,
            cwd: true,
            argv: true,
            git_commit: true,
            env_allowlist: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RedactSection {
    pub custom_patterns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BufferSection {
    pub enabled: bool,
    pub batch_size: usize,
    pub flush_interval_ms: u64,
    pub queue_capacity: usize,
}

impl Default for BufferSection {
    fn default() -> Self {
        BufferSection {
            enabled: false,
            batch_size: 50,
            flush_interval_ms: 1000,
            queue_capacity: 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TaxonomySection {
    pub auto_classify_enabled: bool,
    pub custom_rules_file: Option<PathBuf>,
}

impl Default for TaxonomySection {
    fn default() -> Self {
        TaxonomySection {
            auto_classify_enabled: true,
            custom_rules_file: None,
        }
    }
}

/// Config for the P10c file notifier (`witslog_plugin::Notifier`, wired in
/// `witslog-runtime`). Webhook/desktop notifiers deliberately not offered
/// here — `witslog-runtime` links into `witslog-ffi`, which is `dlopen`'d
/// into every host process, so adding an HTTP client dependency there was
/// rejected; anyone wanting a webhook implements `Notifier` themselves.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NotifySection {
    pub enabled: bool,
    /// Minimum severity that triggers a notification (`Severity::as_str()` values).
    pub min_severity: String,
    /// NDJSON append target for the builtin file notifier.
    pub path: Option<PathBuf>,
    /// Suppress repeat notifications for the same fingerprint within this
    /// many seconds. `None`/`0` = no throttle.
    pub once_per_fingerprint_secs: Option<u64>,
}

impl Default for NotifySection {
    fn default() -> Self {
        NotifySection {
            enabled: false,
            min_severity: "error".to_string(),
            path: None,
            once_per_fingerprint_secs: None,
        }
    }
}

/// FR-P9-004 wiring: opt-in `metadata`-field encryption (see
/// `witslog_core::crypto::FieldCipher`). `key_env` names an env var holding
/// a 64-char hex AES-256-GCM key — the key itself is never stored in
/// `config.toml`. `None` (default) = encryption off. Level-0 key model
/// (single active key, no rotation) — see CLAUDE.md/CONTRACT.md for the
/// rotate-by-retention-or-reimport guidance.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CryptoSection {
    pub key_env: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config file {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse config file {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
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
            enrich: EnrichSection::default(),
            redact: RedactSection::default(),
            buffer: BufferSection::default(),
            taxonomy: TaxonomySection::default(),
            notify: NotifySection::default(),
            crypto: CryptoSection::default(),
        }
    }

    /// Loads config from a TOML file. Structural parsing only — pattern-compile
    /// validation of `redact.custom_patterns` happens where the caller constructs
    /// a `witslog_core::Redactor` (CLI/FFI), so an invalid regex is reported there.
    pub fn load_from_file(path: &Path) -> Result<Config, ConfigError> {
        let content = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        toml::from_str(&content).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Loads `<cwd-resolved-project>/.witslog/config.toml` if present, else
    /// falls back to `Config::default_project()`.
    pub fn load_or_default(cwd: &Path) -> Config {
        let project_dir = resolve_project_db(cwd)
            .parent()
            .map(|p| p.to_path_buf());
        if let Some(dir) = project_dir {
            let config_path = dir.join("config.toml");
            if config_path.exists() {
                if let Ok(cfg) = Config::load_from_file(&config_path) {
                    return cfg;
                }
            }
        }
        Config::default_project()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notify_defaults_to_disabled() {
        let cfg = Config::default_project();
        assert!(!cfg.notify.enabled);
        assert_eq!(cfg.notify.min_severity, "error");
        assert_eq!(cfg.notify.path, None);
    }

    #[test]
    fn notify_section_parses_from_toml() {
        let toml_str = r#"
            [notify]
            enabled = true
            min_severity = "warn"
            path = "/tmp/witslog-notify.ndjson"
            once_per_fingerprint_secs = 300
        "#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert!(cfg.notify.enabled);
        assert_eq!(cfg.notify.min_severity, "warn");
        assert_eq!(cfg.notify.path, Some(PathBuf::from("/tmp/witslog-notify.ndjson")));
        assert_eq!(cfg.notify.once_per_fingerprint_secs, Some(300));
    }

    #[test]
    fn config_without_notify_section_uses_defaults() {
        let cfg: Config = toml::from_str("").unwrap();
        assert!(!cfg.notify.enabled);
    }

    #[test]
    fn crypto_defaults_to_disabled() {
        let cfg = Config::default_project();
        assert_eq!(cfg.crypto.key_env, None);
    }

    #[test]
    fn crypto_section_parses_from_toml() {
        let toml_str = r#"
            [crypto]
            key_env = "WITSLOG_ENCRYPTION_KEY"
        "#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.crypto.key_env, Some("WITSLOG_ENCRYPTION_KEY".to_string()));
    }

    #[test]
    fn config_without_crypto_section_uses_defaults() {
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(cfg.crypto.key_env, None);
    }
}
