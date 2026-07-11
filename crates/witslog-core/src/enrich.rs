use crate::event::EventBuilder;
use std::path::{Path, PathBuf};

/// Controls which runtime fields are auto-captured into an event's context.
/// Every field is best-effort: a failure to capture one never affects the others
/// and never fails the build.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct EnrichConfig {
    pub hostname: bool,
    pub pid: bool,
    pub cwd: bool,
    pub argv: bool,
    pub git_commit: bool,
    pub env_allowlist: Vec<String>,
}

impl Default for EnrichConfig {
    fn default() -> Self {
        EnrichConfig {
            hostname: true,
            pid: true,
            cwd: true,
            argv: true,
            git_commit: true,
            env_allowlist: Vec::new(),
        }
    }
}

/// Auto-populates hostname/pid/cwd/argv/git_commit/env into the builder per `cfg`.
/// Existing values the caller already set (via `.hostname()` or `.context()`) win.
pub fn enrich(mut builder: EventBuilder, cfg: &EnrichConfig) -> EventBuilder {
    if cfg.hostname && builder.hostname.is_none() {
        if let Some(host) = read_hostname() {
            builder.hostname = Some(host);
        }
    }

    let mut ctx = match builder.context.take() {
        Some(serde_json::Value::Object(map)) => map,
        Some(other) => {
            // Non-object context set by the caller: leave it untouched, skip merge.
            builder.context = Some(other);
            return builder;
        }
        None => serde_json::Map::new(),
    };

    if cfg.pid && !ctx.contains_key("pid") {
        ctx.insert("pid".to_string(), serde_json::json!(std::process::id()));
    }

    if cfg.cwd && !ctx.contains_key("cwd") {
        if let Ok(cwd) = std::env::current_dir() {
            ctx.insert("cwd".to_string(), serde_json::json!(cwd.display().to_string()));
        }
    }

    if cfg.argv && !ctx.contains_key("argv") {
        let argv: Vec<String> = std::env::args().collect();
        if !argv.is_empty() {
            ctx.insert("argv".to_string(), serde_json::json!(argv));
        }
    }

    if cfg.git_commit && !ctx.contains_key("git_commit") {
        if let Ok(cwd) = std::env::current_dir() {
            if let Some(sha) = read_git_head_short(&cwd) {
                ctx.insert("git_commit".to_string(), serde_json::json!(sha));
            }
        }
    }

    if !cfg.env_allowlist.is_empty() {
        let mut env_obj = serde_json::Map::new();
        for name in &cfg.env_allowlist {
            if let Ok(val) = std::env::var(name) {
                env_obj.insert(name.clone(), serde_json::json!(val));
            }
        }
        if !env_obj.is_empty() {
            ctx.insert("env".to_string(), serde_json::Value::Object(env_obj));
        }
    }

    if !ctx.is_empty() {
        builder.context = Some(serde_json::Value::Object(ctx));
    }

    builder
}

fn read_hostname() -> Option<String> {
    hostname::get().ok()?.into_string().ok()
}

/// Walk up from `start` looking for a `.git` dir (mirrors witslog-config's
/// project-marker walk-up), then resolve HEAD to a short commit SHA via plain
/// file reads — no `git` subprocess spawn.
fn read_git_head_short(start: &Path) -> Option<String> {
    let git_dir = find_git_dir(start)?;
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();

    let sha = if let Some(ref_path) = head.strip_prefix("ref: ") {
        std::fs::read_to_string(git_dir.join(ref_path))
            .ok()
            .map(|s| s.trim().to_string())
            .or_else(|| read_packed_ref(&git_dir, ref_path))?
    } else {
        head.to_string()
    };

    if sha.len() < 7 {
        return None;
    }
    Some(sha[..7].to_string())
}

fn read_packed_ref(git_dir: &Path, ref_path: &str) -> Option<String> {
    let packed = std::fs::read_to_string(git_dir.join("packed-refs")).ok()?;
    for line in packed.lines() {
        if let Some((sha, name)) = line.split_once(' ') {
            if name == ref_path {
                return Some(sha.to_string());
            }
        }
    }
    None
}

fn find_git_dir(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        let candidate = current.join(".git");
        if candidate.is_dir() {
            return Some(candidate);
        }
        if !current.pop() {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::EventBuilder;

    fn init_git_repo(dir: &Path, sha: &str) {
        let git_dir = dir.join(".git");
        std::fs::create_dir_all(git_dir.join("refs/heads")).unwrap();
        std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(git_dir.join("refs/heads/main"), format!("{sha}\n")).unwrap();
    }

    #[test]
    fn enriches_git_commit_when_repo_present() {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path(), "abcdef1234567890abcdef1234567890abcdef12");

        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let cfg = EnrichConfig::default();
        let builder = enrich(EventBuilder::new("app", "msg"), &cfg);
        let event = builder.build();

        std::env::set_current_dir(orig).unwrap();

        let ctx = event.context.expect("context set");
        assert_eq!(ctx["git_commit"], "abcdef1");
        assert!(ctx.get("pid").is_some());
        assert!(ctx.get("cwd").is_some());
    }

    #[test]
    fn omits_git_commit_when_no_repo() {
        let dir = tempfile::tempdir().unwrap();
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let cfg = EnrichConfig::default();
        let builder = enrich(EventBuilder::new("app", "msg"), &cfg);
        let event = builder.build();

        std::env::set_current_dir(orig).unwrap();

        let ctx = event.context.expect("context set (pid/cwd still present)");
        assert!(ctx.get("git_commit").is_none());
    }

    #[test]
    fn disabled_field_is_omitted() {
        let mut cfg = EnrichConfig::default();
        cfg.pid = false;
        cfg.cwd = false;
        cfg.argv = false;
        cfg.git_commit = false;
        cfg.hostname = false;

        let builder = enrich(EventBuilder::new("app", "msg"), &cfg);
        let event = builder.build();

        assert!(event.hostname.is_none());
        assert!(event.context.is_none());
    }

    #[test]
    fn env_allowlist_captures_named_vars() {
        std::env::set_var("WITSLOG_TEST_ENRICH_VAR", "hello");
        let mut cfg = EnrichConfig::default();
        cfg.pid = false;
        cfg.cwd = false;
        cfg.argv = false;
        cfg.git_commit = false;
        cfg.hostname = false;
        cfg.env_allowlist = vec!["WITSLOG_TEST_ENRICH_VAR".to_string()];

        let builder = enrich(EventBuilder::new("app", "msg"), &cfg);
        let event = builder.build();
        std::env::remove_var("WITSLOG_TEST_ENRICH_VAR");

        let ctx = event.context.expect("context set");
        assert_eq!(ctx["env"]["WITSLOG_TEST_ENRICH_VAR"], "hello");
    }
}
