use clap::{Parser, Subcommand};
use std::path::PathBuf;
use witslog_config::Config;
use witslog_core::{EventBuilder, Severity};
use witslog_store::{DeleteFilter, Store};

mod style;
use style::ColorMode;

#[derive(Parser)]
#[command(name = "witslog")]
#[command(version)]
#[command(about = "AI-native error logging framework", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    #[arg(global = true, short, long)]
    db: Option<PathBuf>,

    /// Emit full structured JSON instead of the default human-readable summary.
    /// Applies to `get` and `query` — the two read paths that otherwise hide
    /// captured `context`/`tags`/`stacktrace`/`error_code`/`correlation_id`
    /// behind a bare `id [app] Severity :: message` line.
    #[arg(global = true, long)]
    json: bool,

    /// Colorize `get`/`query` output: `auto` (default) colorizes only on a
    /// real TTY and honors `NO_COLOR`; `always`/`never` override detection.
    /// Never affects `--json`, which stays byte-identical for scripts/CI.
    #[arg(global = true, long, value_enum, default_value_t = ColorMode::Auto)]
    color: ColorMode,
}

#[derive(Subcommand)]
enum Commands {
    Init {
        #[arg(value_name = "PATH", default_value = ".")]
        path: PathBuf,
        /// Turn on encryption for sensitive data without the interactive wizard.
        /// Takes the name of the environment variable that will hold the key
        /// (defaults to WITSLOG_ENCRYPTION_KEY if you just pass --encrypt with
        /// no name). Skips all prompts.
        #[arg(long, num_args = 0..=1, default_missing_value = "WITSLOG_ENCRYPTION_KEY")]
        encrypt: Option<String>,
        /// Skip the interactive wizard even on a real terminal — use flags/defaults only.
        #[arg(long)]
        yes: bool,
    },
    Log {
        app: String,
        message: String,
        #[arg(long)]
        version: Option<String>,
        #[arg(long)]
        environment: Option<String>,
        #[arg(long)]
        severity: Option<String>,
        #[arg(long)]
        category: Option<String>,
        #[arg(long)]
        error_code: Option<String>,
        #[arg(long)]
        exception: Option<String>,
    },
    Get {
        event_id: String,
    },
    #[command(short_flag = 'q')]
    Query {
        #[arg(value_name = "FTS_QUERY")]
        text: String,
        #[arg(long)]
        application: Option<String>,
        #[arg(long)]
        category: Option<String>,
        #[arg(long)]
        severity_min: Option<String>,
        /// Only show events with `resolved_at IS NULL`.
        #[arg(long)]
        unresolved: bool,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
    },
    Stats {
        #[arg(long)]
        application: Option<String>,
        #[arg(long)]
        severity_min: Option<String>,
        /// Show fingerprint-level mean time-to-resolution instead of the
        /// normal stats summary.
        #[arg(long)]
        mttr: bool,
    },
    Export {
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        format: Option<String>,
    },
    Import {
        path: PathBuf,
    },
    Vacuum,
    Prune {
        #[arg(long)]
        older_than: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    Config {
        /// "encrypt" jumps straight to the encryption prompt/change instead of
        /// the default resolved-config printout. Omit on a real terminal to
        /// get an arrow-key menu instead.
        action: Option<String>,
    },
    Archive {
        #[arg(long)]
        older_than: Option<String>,
    },
    Backup {
        output: PathBuf,
        #[arg(long)]
        force: bool,
    },
    ListDbs,
    Migrate,
    Resolve {
        event_id: String,
        /// Move `resolved_at` even if already resolved (default: first
        /// resolution wins).
        #[arg(long)]
        force: bool,
    },
    Delete {
        #[arg(long)]
        event_id: Option<String>,
        #[arg(long)]
        fingerprint: Option<String>,
        #[arg(long)]
        resolved_before: Option<String>,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        dry_run: bool,
    },
    Doctor {
        /// FR-P9-007: recompute the audit hash chain and report any break
        /// (offending row + expected/actual hash) instead of the normal
        /// health summary.
        #[arg(long)]
        verify_audit: bool,
    },
    Category {
        #[command(subcommand)]
        action: CategoryAction,
    },
    /// Run an MCP (Model Context Protocol) server over stdio (P5).
    ServeMcp {
        /// Serve over stdio (currently the only supported transport; kept
        /// explicit so future `--http` doesn't silently change defaults).
        #[arg(long)]
        stdio: bool,
        /// Additional project DBs to attach for the opt-in `search_all` tool.
        #[arg(long)]
        attach: Vec<PathBuf>,
        /// Enable the write-capable `witslog_delete` tool. Off by default —
        /// the server is otherwise strictly read-only (FR-P5-005).
        #[arg(long)]
        allow_write: bool,
        /// Print a generic `mcpServers` JSON snippet for MCP clients and exit
        /// (FR-P8-004). Does not open or require a DB.
        #[arg(long)]
        print_mcp_config: bool,
    },
    /// Remove the witslog binary and, with --purge, its data files (FR-P8-006).
    Uninstall {
        /// Also delete the resolved project `.witslog/` directory and the
        /// global config directory. Without this flag only the binary is
        /// targeted for removal.
        #[arg(long)]
        purge: bool,
    },
}

#[derive(Subcommand)]
enum CategoryAction {
    /// Register a custom category (builtin=0). Rejects if it collides with a builtin canonical.
    Add {
        canonical: String,
        #[arg(long)]
        parent: Option<String>,
        #[arg(long)]
        label: Option<String>,
    },
    /// Register an alias pointing at an existing canonical.
    Alias {
        alias: String,
        canonical: String,
    },
    /// List the full category tree.
    List,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // MUST write to stderr, never stdout: `serve-mcp --stdio` treats stdout as
    // a pure JSON-RPC channel end to end (transport.rs has no println!/print
    // of its own) — any stray tracing line on stdout (from this crate or a
    // dependency) gets parsed by the MCP client as a malformed response
    // (Zod's invalid_union error trying every response shape), breaking every
    // subsequent message on that connection. This applies to every subcommand
    // since the subscriber is installed once here, before the match on
    // `cli.command`.
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(std::io::stderr)
        .init();

    // Load `.witslog/.env` (e.g. WITSLOG_ENCRYPTION_KEY written by the
    // encryption wizard) before anything else reads env vars — must run
    // before witslog_runtime::init_default() below, which resolves
    // crypto.key_env for its own write pipeline.
    load_dotenv_if_present();

    // Mount witslog so the CLI's own panics are auto-captured as Fatal events.
    let _witslog_guard = witslog_runtime::init_default();

    let cli = Cli::parse();

    match cli.command {
        Commands::Init { path, encrypt, yes } => {
            init_db(&path, encrypt, yes)?;
        }
        Commands::Log {
            app,
            message,
            version,
            environment,
            severity,
            category,
            error_code,
            exception,
        } => {
            log_event(
                &cli.db, &app, &message, version, environment, severity, category, error_code,
                exception,
            )?;
        }
        Commands::Get { event_id } => {
            get_event(&cli.db, &event_id, cli.json, style::should_colorize(cli.color))?;
        }
        Commands::Query {
            text,
            application,
            category,
            severity_min,
            unresolved,
            limit,
            cursor,
        } => {
            query_search(
                &cli.db, &text, application, category, severity_min, unresolved, limit, cursor,
                cli.json, style::should_colorize(cli.color),
            )?;
        }
        Commands::Stats { application, severity_min, mttr } => {
            if mttr {
                cmd_mttr(&cli.db, application, severity_min)?;
            } else {
                cmd_stats(&cli.db, application, severity_min)?;
            }
        }
        Commands::Export { output, format } => {
            cmd_export(&cli.db, output, format)?;
        }
        Commands::Import { path } => {
            cmd_import(&cli.db, &path)?;
        }
        Commands::Vacuum => {
            cmd_vacuum(&cli.db)?;
        }
        Commands::Prune { older_than, dry_run } => {
            cmd_prune(&cli.db, older_than, dry_run)?;
        }
        Commands::Config { action } => {
            cmd_config(&cli.db, action)?;
        }
        Commands::Archive { older_than } => {
            cmd_archive(&cli.db, older_than)?;
        }
        Commands::Backup { output, force } => {
            cmd_backup(&cli.db, &output, force)?;
        }
        Commands::ListDbs => {
            cmd_list_dbs()?;
        }
        Commands::Migrate => {
            cmd_migrate(&cli.db)?;
        }
        Commands::Resolve { event_id, force } => {
            resolve_event(&cli.db, &event_id, force)?;
        }
        Commands::Delete {
            event_id,
            fingerprint,
            resolved_before,
            force,
            dry_run,
        } => {
            delete_events(&cli.db, event_id, fingerprint, resolved_before, force, dry_run)?;
        }
        Commands::Doctor { verify_audit } => {
            doctor(verify_audit)?;
        }
        Commands::Category { action } => {
            cmd_category(&cli.db, action)?;
        }
        Commands::ServeMcp {
            stdio: _,
            attach,
            allow_write,
            print_mcp_config,
        } => {
            if print_mcp_config {
                print_mcp_config_snippet()?;
            } else {
                cmd_serve_mcp(&cli.db, attach, allow_write)?;
            }
        }
        Commands::Uninstall { purge } => {
            cmd_uninstall(purge)?;
        }
    }

    Ok(())
}

fn init_db(
    path: &PathBuf,
    encrypt: Option<String>,
    non_interactive: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let witslog_dir = path.join(".witslog");
    std::fs::create_dir_all(&witslog_dir)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o700);
        std::fs::set_permissions(&witslog_dir, perms)?;
    }

    let db_path = witslog_dir.join("witslog.db");
    let _store = Store::open_or_create(&db_path)?;

    // FR-P9-005: restrict DB file permissions (0600) alongside the 0700 dir
    // above. Windows has no POSIX mode bits; ACL hardening is out of scope
    // here (same call the project already made for the dir, above).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&db_path, perms)?;
    }

    println!("✓ Initialized witslog at {}", db_path.display());
    println!("  DB path: {}", db_path.display());

    // Only offer the wizard on a real terminal, and only when the caller
    // hasn't already told us what they want via --encrypt/--yes — a flag
    // means "skip prompts, I've decided already" (keeps CI/scripts working
    // without ever blocking on stdin: `witslog init --encrypt` still passes
    // straight through with no prompts).
    let wizard_eligible = !non_interactive && encrypt.is_none() && is_interactive_terminal();

    let encrypt_choice = if wizard_eligible {
        run_init_encryption_prompt()?
    } else {
        encrypt
    };

    if let Some(var_name) = encrypt_choice {
        enable_metadata_encryption(&witslog_dir, &var_name)?;
    }

    Ok(())
}

/// True only when both stdin and stdout are a real terminal — piping either
/// end (CI, `| tee`, a script) must never trigger a prompt that would hang
/// waiting for input that will never arrive.
fn is_interactive_terminal() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

/// The one fixed name every wizard-driven setup uses for the encryption-key
/// env var — no naming prompt, no bikeshedding. Anyone who wants a different
/// name can still set `[crypto] key_env = "..."` in config.toml by hand; the
/// wizard just doesn't ask.
const DEFAULT_ENCRYPTION_KEY_ENV: &str = "WITSLOG_ENCRYPTION_KEY";

/// Arrow-key/space/enter prompt asking whether to turn on metadata
/// encryption. Returns `None` if the user declines or cancels (Ctrl+C);
/// `Some(DEFAULT_ENCRYPTION_KEY_ENV)` if they say yes.
fn run_init_encryption_prompt() -> Result<Option<String>, Box<dyn std::error::Error>> {
    use dialoguer::{theme::ColorfulTheme, MultiSelect};

    println!();
    println!("A quick question to set up your project (press Ctrl+C anytime to skip — nothing is written until you confirm):");
    println!();

    let theme = ColorfulTheme::default();
    let options = ["Protect sensitive info you log (like emails, tokens, or account numbers) — recommended if you're not sure"];

    let selected = match MultiSelect::with_theme(&theme)
        .with_prompt("Optional features (space to select, enter to continue)")
        .items(&options)
        .interact_opt()
    {
        Ok(Some(selected)) => selected,
        Ok(None) | Err(_) => return Ok(None), // Ctrl+C / cancelled — skip silently
    };

    if selected.is_empty() {
        return Ok(None);
    }

    println!();
    println!("This encrypts one field (\"metadata\") so it's unreadable without a secret key you keep yourself.");
    println!("Everything else you log stays searchable as normal — this only protects data you deliberately mark sensitive.");

    Ok(Some(DEFAULT_ENCRYPTION_KEY_ENV.to_string()))
}

/// Generates a fresh AES-256-GCM key, writes only the *name* of the env var
/// that will hold it into config.toml (never the key itself — see
/// bindings/CONTRACT.md's "Metadata encryption" section), and writes the
/// actual key value into `.witslog/.env` (gitignored via the blanket
/// `.witslog/` entry in `.gitignore`) so no manual copy-paste/export step is
/// needed — `load_dotenv_if_present` (called once at CLI startup, before any
/// command runs) reads it back into the process env on every subsequent
/// invocation. Uses `toml_edit` rather than a blind rewrite so a pre-existing
/// config.toml's other sections/comments are left untouched.
fn enable_metadata_encryption(
    witslog_dir: &std::path::Path,
    var_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let key_hex = generate_hex_key();
    let config_path = witslog_dir.join("config.toml");

    let mut doc: toml_edit::DocumentMut = if config_path.exists() {
        std::fs::read_to_string(&config_path)?.parse()?
    } else {
        toml_edit::DocumentMut::new()
    };
    // Explicit `Table` (not the inline-table shorthand `crypto = { key_env = ... }`
    // that indexing an empty slot defaults to) so the file reads as the
    // `[crypto]` section documented in bindings/CONTRACT.md.
    if doc.get("crypto").and_then(|i| i.as_table()).is_none() {
        doc["crypto"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    doc["crypto"]["key_env"] = toml_edit::value(var_name);
    std::fs::write(&config_path, doc.to_string())?;

    write_env_file_var(witslog_dir, var_name, &key_hex)?;
    std::env::set_var(var_name, &key_hex); // takes effect immediately, this process too

    println!();
    println!("✓ Encryption turned on (config.toml now points at env var: {var_name})");
    println!("✓ Key written to {} — never committed (gitignored), auto-loaded on every witslog run", witslog_dir.join(".env").display());
    println!("  No manual export needed. Losing this file means anything already encrypted");
    println!("  stays locked forever, so back it up somewhere safe (password manager, vault).");
    println!();

    Ok(())
}

/// Writes/replaces a single `KEY=value` line in `<witslog_dir>/.env`,
/// preserving any other vars already there. Creates the file (0600 on Unix)
/// if it doesn't exist yet.
fn write_env_file_var(
    witslog_dir: &std::path::Path,
    var_name: &str,
    value: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let env_path = witslog_dir.join(".env");

    let mut lines: Vec<String> = if env_path.exists() {
        std::fs::read_to_string(&env_path)?
            .lines()
            .filter(|line| !line.starts_with(&format!("{var_name}=")))
            .map(|line| line.to_string())
            .collect()
    } else {
        Vec::new()
    };
    lines.push(format!("{var_name}={value}"));
    std::fs::write(&env_path, lines.join("\n") + "\n")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&env_path, std::fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}

/// Loads `<cwd>/.witslog/.env` (if present) into the process environment,
/// called once at CLI startup before any subcommand runs. Only fills in vars
/// that aren't already set in the real environment — an explicit `export`/
/// `$env:` from the caller's shell always wins over the file. Silently a
/// no-op if the project isn't initialized yet or the file doesn't exist.
fn load_dotenv_if_present() {
    let Ok(cwd) = std::env::current_dir() else { return };
    let env_path = cwd.join(".witslog").join(".env");
    let Ok(contents) = std::fs::read_to_string(&env_path) else { return };

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            if std::env::var_os(key).is_none() {
                std::env::set_var(key, value);
            }
        }
    }
}

/// 32 random bytes, hex-encoded — the shape `FieldCipher::from_env` expects.
fn generate_hex_key() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn log_event(
    db_override: &Option<PathBuf>,
    app: &str,
    message: &str,
    version: Option<String>,
    environment: Option<String>,
    severity_str: Option<String>,
    category: Option<String>,
    error_code: Option<String>,
    exception: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir()?;

    let config = Config::load_or_default(&cwd);
    let db_path = db_override.clone().unwrap_or_else(|| config.resolve_db_path(&cwd));

    if !db_path.parent().map(|p| p.exists()).unwrap_or(false) {
        return Err(format!(
            "Database not initialized. Run 'witslog init' in {}",
            cwd.display()
        )
        .into());
    }

    let severity = match severity_str.as_deref().unwrap_or("error") {
        "trace" => Severity::Trace,
        "debug" => Severity::Debug,
        "info" => Severity::Info,
        "warn" => Severity::Warn,
        "error" => Severity::Error,
        "critical" => Severity::Critical,
        "fatal" => Severity::Fatal,
        _ => Severity::Error,
    };

    let mut builder = EventBuilder::new(app, message).severity(severity);

    if let Some(v) = version {
        builder = builder.version(v);
    }
    if let Some(e) = environment {
        builder = builder.environment(e);
    }
    if let Some(c) = category {
        builder = builder.category(c);
    }
    if let Some(ec) = error_code {
        builder = builder.error_code(ec);
    }
    if let Some(exc) = exception {
        builder = builder.exception(exc);
    }

    // Single home for the enrich → redact → classify → build → write pipeline;
    // previously ~100 lines of boilerplate inlined here.
    let event = witslog_runtime::build_and_write(&config, &db_path, builder)?;

    println!("✓ Event logged");
    println!("  event_id: {}", event.event_id);
    println!("  fingerprint: {}", event.fingerprint);
    println!("  DB: {}", db_path.display());

    Ok(())
}

fn get_event(
    db_override: &Option<PathBuf>,
    event_id: &str,
    json: bool,
    colorize: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir()?;

    // `load_or_default` so `[crypto] key_env` is respected — see the same
    // change/rationale in `cmd_serve_mcp`.
    let config = Config::load_or_default(&cwd);
    let db_path = db_override.clone().unwrap_or_else(|| config.resolve_db_path(&cwd));

    let store = Store::open_or_create(&db_path)?;
    let writer = witslog_store::EventWriter::new(store.conn());

    match writer.query_by_id(event_id)? {
        Some(mut event) => {
            // FR-P9-004: display-only decrypt (never fail-closed on read) —
            // decrypts `metadata` if this process has the configured key,
            // else replaces it with the `"<encrypted>"` placeholder.
            let cipher = config
                .crypto
                .key_env
                .as_ref()
                .and_then(|var| witslog_core::FieldCipher::from_env(var).ok().flatten());
            event.metadata =
                witslog_core::decrypt_metadata_for_display(event.metadata, cipher.as_ref());

            if json {
                println!("{}", serde_json::to_string_pretty(&event)?);
                return Ok(());
            }
            println!(
                "Event found:  {}  {}",
                style::severity_chip(event.severity, colorize),
                style::status_badge(event.resolved_at.is_some(), colorize)
            );
            println!("  event_id: {}", event.event_id);
            println!("  timestamp: {}", event.timestamp);
            println!("  application: {}", event.application);
            println!("  message: {}", event.message);
            println!("  severity: {}", event.severity.as_str());
            println!("  fingerprint: {}", event.fingerprint);
            if let Some(cat) = &event.category {
                println!("  category: {}", cat);
            }
            if let Some(code) = &event.error_code {
                println!("  error_code: {}", style::dim(code, colorize));
            }
            if let Some(exc) = &event.exception {
                println!("  exception: {}", exc);
            }
            if let Some(cid) = &event.correlation_id {
                println!("  correlation_id: {}", cid);
            }
            if let Some(pid) = &event.parent_event_id {
                println!("  parent_event_id: {}", pid);
            }
            if let Some(env) = &event.environment {
                println!("  environment: {}", env);
            }
            if let Some(ver) = &event.version {
                println!("  version: {}", ver);
            }
            if let Some(tags) = &event.tags {
                if !tags.is_empty() {
                    println!("  tags: {}", style::dim(&tags.join(", "), colorize));
                }
            }
            if let Some(ctx) = &event.context {
                println!("  context: {}", ctx);
            }
            if let Some(meta) = &event.metadata {
                println!("  metadata: {}", meta);
            }
            if let Some(trace) = &event.stacktrace {
                println!("  stacktrace:");
                for line in trace.lines() {
                    println!("    {}", line);
                }
            }
            match &event.resolved_at {
                Some(r) => println!("  resolved_at: {}", r),
                None => println!("  resolved_at: (unresolved)"),
            }
        }
        None => {
            if json {
                println!("null");
            } else {
                println!("Event not found: {}", event_id);
            }
        }
    }

    Ok(())
}

fn query_search(
    db_override: &Option<PathBuf>,
    text: &str,
    application: Option<String>,
    category: Option<String>,
    severity_min: Option<String>,
    unresolved: bool,
    limit: usize,
    cursor: Option<String>,
    json: bool,
    colorize: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir()?;

    let config = Config::default_project();
    let db_path = db_override.clone().unwrap_or_else(|| config.resolve_db_path(&cwd));

    let store = Store::open_or_create(&db_path)?;
    let conn = store.conn().conn();
    let search = witslog_query::SearchEngine::new(&conn);

    let filters = witslog_query::Filters {
        application,
        category,
        severity_min,
        resolved: if unresolved { Some(false) } else { None },
        ..Default::default()
    };

    let result = search.search(text, &filters, limit, cursor, true)?;

    if json {
        // Full structured events — nothing hidden behind the summary line
        // below (no context/tags/stacktrace/error_code/correlation_id lost).
        let out = serde_json::json!({
            "items": result.items,
            "total_estimate": result.total_estimate,
            "next_cursor": result.next_cursor,
            "cursor_warning": result.cursor_warning,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    if let Some(warning) = &result.cursor_warning {
        eprintln!("warning: {}", warning);
    }

    if result.items.is_empty() {
        println!("No matching events.");
    }
    for event in &result.items {
        // Summary line stays terse for scanability; error_code + tags are
        // appended when present since they're the highest-signal fields for
        // triage (full detail — context/stacktrace/etc — needs --json).
        let mut line = format!(
            "{}  [{}] {}  {} :: {}",
            event.event_id,
            event.application,
            style::severity_chip(event.severity, colorize),
            style::status_badge(event.resolved_at.is_some(), colorize),
            event.message
        );
        if let Some(code) = &event.error_code {
            line.push_str(&format!("  {}", style::dim(&format!("[{}]", code), colorize)));
        }
        if let Some(tags) = &event.tags {
            if !tags.is_empty() {
                line.push_str(&format!(
                    "  {}",
                    style::dim(&format!("#{}", tags.join(" #")), colorize)
                ));
            }
        }
        println!("{}", line);
    }
    println!("\n{} match(es) (showing {})", result.total_estimate, result.items.len());
    if let Some(next) = result.next_cursor {
        println!("next cursor: {}", next);
    }
    println!("(use --json for full context/tags/stacktrace/correlation_id)");

    Ok(())
}

fn resolve_event(
    db_override: &Option<PathBuf>,
    event_id: &str,
    force: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir()?;

    let config = Config::default_project();
    let db_path = db_override.clone().unwrap_or_else(|| config.resolve_db_path(&cwd));

    let store = Store::open_or_create(&db_path)?;
    let writer = witslog_store::EventWriter::new(store.conn());

    if !writer.mark_resolved(event_id, force)? {
        eprintln!(
            "Error: no unresolved event matched '{}' (unknown id, or already resolved — pass --force to move resolved_at)",
            event_id
        );
        std::process::exit(2);
    }

    println!("✓ Event resolved");
    println!("  event_id: {}", event_id);

    Ok(())
}

fn delete_events(
    db_override: &Option<PathBuf>,
    event_id: Option<String>,
    fingerprint: Option<String>,
    resolved_before: Option<String>,
    force: bool,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir()?;

    let config = Config::default_project();
    let db_path = db_override.clone().unwrap_or_else(|| config.resolve_db_path(&cwd));

    let store = Store::open_or_create(&db_path)?;
    let writer = witslog_store::EventWriter::new(store.conn());

    let filter = DeleteFilter {
        event_id,
        fingerprint,
        resolved_before,
        force,
    };

    if dry_run {
        // Actually runs the same SELECT `delete_resolved` would, so the count/ids
        // shown are real — previously this just echoed the filter back unevaluated,
        // which looked identical whether 0 or 1000 rows would have matched.
        let would_delete_ids = writer.preview_delete(&filter)?;
        println!(
            "(dry run — no rows deleted; re-run without --dry-run to apply)"
        );
        println!("  filter: {:?}", filter);
        println!("  would delete {} event(s)", would_delete_ids.len());
        for id in &would_delete_ids {
            println!("  {}", id);
        }
        return Ok(());
    }

    let deleted_ids = writer.delete_resolved(&filter)?;

    println!("✓ Deleted {} event(s)", deleted_ids.len());
    for id in &deleted_ids {
        println!("  {}", id);
    }

    Ok(())
}

fn doctor(verify_audit: bool) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir()?;
    let config = Config::default_project();
    let db_path = config.resolve_db_path(&cwd);

    println!("witslog doctor");
    println!("  witslog version: {}", env!("CARGO_PKG_VERSION"));
    println!("  max supported schema version: {}", witslog_store::CURRENT_SCHEMA_VERSION);
    println!("  cwd: {}", cwd.display());
    println!("  resolved db: {}", db_path.display());
    println!("  db exists: {}", db_path.exists());

    if !db_path.exists() {
        return Ok(());
    }

    if verify_audit {
        let store = Store::open_or_create(&db_path)?;
        let conn = store.conn().conn();
        match witslog_store::audit::verify_chain(&conn)? {
            witslog_store::AuditVerifyResult::Ok { rows_checked, tombstones_bridged } => {
                println!("  ✓ audit chain verified ({} rows, no tampering detected)", rows_checked);
                if tombstones_bridged > 0 {
                    println!(
                        "    ({} row(s) previously removed by delete/prune/archive; bridged via tombstone, not tampering)",
                        tombstones_bridged
                    );
                }
            }
            witslog_store::AuditVerifyResult::Broken(b) => {
                println!(
                    "  ✗ audit chain broken at row id={} event_id={}",
                    b.row_id, b.event_id
                );
                println!("    expected hash: {}", b.expected_hash);
                println!(
                    "    actual hash:   {}",
                    b.actual_hash.as_deref().unwrap_or("<null>")
                );
                std::process::exit(1);
            }
        }
        return Ok(());
    }

    match Store::open_or_create(&db_path) {
        Ok(store) => {
            println!("  ✓ database healthy");
            let writer = witslog_store::EventWriter::new(store.conn());
            if let Ok(dropped) = writer.dropped_count() {
                println!("  dropped events (lifetime): {}", dropped);
            }
        }
        Err(e) => {
            println!("  ✗ database check failed: {}", e);
        }
    }

    Ok(())
}

/// FR-P8-004: emit a generic `mcpServers` snippet for MCP clients, launching
/// `witslog serve-mcp --stdio` with `cwd` = the current project directory.
/// Requires no DB — purely reflects how the running binary would be invoked.
fn print_mcp_config_snippet() -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir()?;
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "witslog".to_string());

    let snippet = serde_json::json!({
        "mcpServers": {
            "witslog": {
                "command": exe,
                "args": ["serve-mcp", "--stdio"],
                "cwd": cwd.display().to_string()
            }
        }
    });

    println!("{}", serde_json::to_string_pretty(&snippet)?);

    Ok(())
}

/// Global config dir per PLAN.md §7 OS conventions (defaults only, not error
/// data): Linux `$XDG_CONFIG_HOME/witslog`, macOS `~/Library/Application
/// Support/witslog`, Windows `%APPDATA%\witslog`.
fn global_config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("witslog"))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join("Library/Application Support/witslog"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .map(|base| base.join("witslog"))
    }
}

/// FR-P8-006: remove the witslog binary and, with `--purge`, its data files.
/// Self-deleting a running executable is platform-dependent: Unix permits
/// unlinking an open file (the process keeps running off the now-unlinked
/// inode until it exits); Windows does not allow deleting a file that is
/// memory-mapped/in-use by the running process, so on Windows we report the
/// path and instruct the user to remove it after the process exits.
fn cmd_uninstall(purge: bool) -> Result<(), Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?;

    println!("witslog uninstall");
    println!("  binary: {}", exe.display());

    #[cfg(unix)]
    {
        std::fs::remove_file(&exe)?;
        println!("  ✓ binary removed");
    }
    #[cfg(windows)]
    {
        println!("  ⚠ cannot delete the running binary on Windows; remove it manually after exit:");
        println!("    del \"{}\"", exe.display());
    }

    if purge {
        let cwd = std::env::current_dir()?;
        for removed in purge_data_dirs(&cwd.join(".witslog"), global_config_dir().as_deref())? {
            println!("  ✓ removed: {}", removed.display());
        }
    } else {
        println!("  (data files left in place; re-run with --purge to remove them)");
    }

    Ok(())
}

/// Pure helper (unit-testable without touching the running binary): removes
/// `project_dir` and `global_dir` if present, returning the paths removed.
fn purge_data_dirs(
    project_dir: &std::path::Path,
    global_dir: Option<&std::path::Path>,
) -> std::io::Result<Vec<PathBuf>> {
    let mut removed = Vec::new();

    if project_dir.exists() {
        std::fs::remove_dir_all(project_dir)?;
        removed.push(project_dir.to_path_buf());
    }

    if let Some(dir) = global_dir {
        if dir.exists() {
            std::fs::remove_dir_all(dir)?;
            removed.push(dir.to_path_buf());
        }
    }

    Ok(removed)
}

#[cfg(test)]
mod uninstall_tests {
    use super::*;

    #[test]
    fn purge_removes_existing_project_and_global_dirs() {
        let tmp = tempfile::TempDir::new().unwrap();
        let project_dir = tmp.path().join(".witslog");
        let global_dir = tmp.path().join("global-config");
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::create_dir_all(&global_dir).unwrap();
        std::fs::write(project_dir.join("witslog.db"), b"x").unwrap();

        let removed = purge_data_dirs(&project_dir, Some(&global_dir)).unwrap();

        assert_eq!(removed.len(), 2);
        assert!(!project_dir.exists());
        assert!(!global_dir.exists());
    }

    #[test]
    fn purge_is_a_noop_when_nothing_exists() {
        let tmp = tempfile::TempDir::new().unwrap();
        let project_dir = tmp.path().join(".witslog");
        let global_dir = tmp.path().join("global-config");

        let removed = purge_data_dirs(&project_dir, Some(&global_dir)).unwrap();

        assert!(removed.is_empty());
    }

    #[test]
    fn purge_without_global_dir_only_removes_project() {
        let tmp = tempfile::TempDir::new().unwrap();
        let project_dir = tmp.path().join(".witslog");
        std::fs::create_dir_all(&project_dir).unwrap();

        let removed = purge_data_dirs(&project_dir, None).unwrap();

        assert_eq!(removed, vec![project_dir.clone()]);
        assert!(!project_dir.exists());
    }
}

fn cmd_stats(
    db_override: &Option<PathBuf>,
    application: Option<String>,
    severity_min: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir()?;
    let config = Config::default_project();
    let db_path = db_override.clone().unwrap_or_else(|| config.resolve_db_path(&cwd));

    let store = Store::open_or_create(&db_path)?;
    let conn = store.conn().conn();
    let agg = witslog_query::AggregateEngine::new(&conn);

    let filters = witslog_query::Filters {
        application,
        severity_min,
        ..Default::default()
    };

    let stats = agg.statistics(&filters)?;

    println!("Statistics");
    println!("  total events: {}", stats.total);
    println!("  unique fingerprints: {}", stats.unique_fingerprints);
    println!("  error rate/day: {:.2}", stats.error_rate_per_day);
    println!("  by severity:");
    for (sev, count) in &stats.by_severity {
        println!("    {}: {}", sev, count);
    }
    println!("  by category:");
    for (cat, count) in &stats.by_category {
        println!("    {}: {}", cat, count);
    }
    println!("  top hosts:");
    for (host, count) in &stats.top_hosts {
        println!("    {}: {}", host, count);
    }

    Ok(())
}

fn cmd_mttr(
    db_override: &Option<PathBuf>,
    application: Option<String>,
    severity_min: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir()?;
    let config = Config::default_project();
    let db_path = db_override.clone().unwrap_or_else(|| config.resolve_db_path(&cwd));

    let store = Store::open_or_create(&db_path)?;
    let conn = store.conn().conn();
    let agg = witslog_query::AggregateEngine::new(&conn);

    let filters = witslog_query::Filters {
        application,
        severity_min,
        ..Default::default()
    };

    let mttr = agg.mttr(&filters)?;

    println!("MTTR (fingerprint-level: first sighting to first fix)");
    println!("  fingerprints resolved: {}", mttr.fingerprints_resolved);
    println!("  fingerprints unresolved: {}", mttr.fingerprints_unresolved);
    match mttr.mean_seconds {
        Some(secs) => println!("  mean time to resolution: {:.1}s ({:.2}h)", secs, secs / 3600.0),
        None => println!("  mean time to resolution: (no resolved fingerprints yet)"),
    }

    Ok(())
}

fn cmd_export(
    db_override: &Option<PathBuf>,
    output: Option<PathBuf>,
    _format: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Write;

    let cwd = std::env::current_dir()?;
    let config = Config::default_project();
    let db_path = db_override.clone().unwrap_or_else(|| config.resolve_db_path(&cwd));

    let store = Store::open_or_create(&db_path)?;
    let writer = witslog_store::EventWriter::new(store.conn());

    let mut sink: Box<dyn Write> = match &output {
        Some(path) => Box::new(std::io::BufWriter::new(std::fs::File::create(path)?)),
        None => Box::new(std::io::BufWriter::new(std::io::stdout())),
    };

    let mut count = 0usize;
    let mut export_err: Option<Box<dyn std::error::Error>> = None;
    writer.for_each_event(None, None, |event| {
        if export_err.is_some() {
            return;
        }
        match serde_json::to_string(&event) {
            Ok(line) => {
                if let Err(e) = writeln!(sink, "{}", line) {
                    export_err = Some(Box::new(e));
                    return;
                }
                count += 1;
            }
            Err(e) => export_err = Some(Box::new(e)),
        }
    })?;

    if let Some(e) = export_err {
        return Err(e);
    }

    sink.flush()?;

    if output.is_some() {
        eprintln!("✓ Exported {} event(s)", count);
    }

    Ok(())
}

fn cmd_import(
    db_override: &Option<PathBuf>,
    path: &PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::BufRead;

    let cwd = std::env::current_dir()?;
    let config = Config::default_project();
    let db_path = db_override.clone().unwrap_or_else(|| config.resolve_db_path(&cwd));

    let store = Store::open_or_create(&db_path)?;
    let writer = witslog_store::EventWriter::new(store.conn());

    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);

    let mut imported = 0usize;
    let mut skipped_dupe = 0usize;
    let mut skipped_malformed = 0usize;

    for (line_no, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        match serde_json::from_str::<witslog_core::Event>(&line) {
            Ok(event) => match writer.write_if_absent(&event) {
                Ok(true) => imported += 1,
                Ok(false) => skipped_dupe += 1,
                Err(e) => {
                    eprintln!("  line {}: write error: {}", line_no + 1, e);
                    skipped_malformed += 1;
                }
            },
            Err(e) => {
                eprintln!("  line {}: malformed: {}", line_no + 1, e);
                skipped_malformed += 1;
            }
        }
    }

    println!("✓ Import complete");
    println!("  imported: {}", imported);
    println!("  skipped (duplicate event_id): {}", skipped_dupe);
    println!("  skipped (malformed): {}", skipped_malformed);

    Ok(())
}

fn cmd_vacuum(db_override: &Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir()?;
    let config = Config::default_project();
    let db_path = db_override.clone().unwrap_or_else(|| config.resolve_db_path(&cwd));

    let store = Store::open_or_create(&db_path)?;
    let conn = store.conn().conn();

    conn.execute_batch("VACUUM;")?;
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE);", [], |_| Ok(()))?;

    println!("✓ Vacuumed and checkpointed WAL");

    Ok(())
}

/// Parse a relative age string like "30d", "24h", "10m" into milliseconds.
fn parse_age_ms(spec: &str) -> Option<i64> {
    let spec = spec.trim();
    if spec.len() < 2 {
        return None;
    }
    let (num_str, unit) = spec.split_at(spec.len() - 1);
    let num: i64 = num_str.parse().ok()?;
    let ms = match unit {
        "d" => num * 24 * 60 * 60 * 1000,
        "h" => num * 60 * 60 * 1000,
        "m" => num * 60 * 1000,
        _ => return None,
    };
    Some(ms)
}

fn cmd_prune(
    db_override: &Option<PathBuf>,
    older_than: Option<String>,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir()?;
    let config = Config::default_project();
    let db_path = db_override.clone().unwrap_or_else(|| config.resolve_db_path(&cwd));

    let older_than = match older_than {
        Some(s) => s,
        None => {
            eprintln!("Error: --older-than is required (e.g. --older-than 30d)");
            std::process::exit(2);
        }
    };

    let age_ms = match parse_age_ms(&older_than) {
        Some(ms) => ms,
        None => {
            eprintln!("Error: invalid --older-than value '{}' (expected e.g. 30d, 24h, 10m)", older_than);
            std::process::exit(2);
        }
    };

    let cutoff_ms = chrono::Utc::now().timestamp_millis() - age_ms;

    let store = Store::open_or_create(&db_path)?;
    let conn = store.conn().conn();

    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE ts_epoch_ms < ?1",
        [cutoff_ms],
        |row| row.get(0),
    )?;

    if dry_run {
        println!("(dry run — no rows deleted)");
        println!("  would delete: {} event(s) older than {}", count, older_than);
        return Ok(());
    }

    let ids: Vec<String> = {
        let mut stmt = conn.prepare("SELECT event_id FROM events WHERE ts_epoch_ms < ?1")?;
        let rows = stmt.query_map([cutoff_ms], |row| row.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    drop(conn);

    // Routed through the store layer's tombstone-then-delete path (not a raw
    // DELETE) so `doctor --verify-audit` can bridge the resulting id gap
    // instead of reporting every later row as tampered (FR-P10-001).
    let writer = witslog_store::EventWriter::new(store.conn());
    writer.delete_by_ids(&ids)?;

    println!("✓ Pruned {} event(s) older than {}", ids.len(), older_than);

    Ok(())
}

fn cmd_config(
    db_override: &Option<PathBuf>,
    action: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir()?;
    let config = Config::load_or_default(&cwd);
    let db_path = db_override.clone().unwrap_or_else(|| config.resolve_db_path(&cwd));

    let print_resolved = || {
        println!("Config resolved:");
        println!("  db path: {}", db_path.display());
        println!("  buffer.enabled: {}", config.buffer.enabled);
        println!("  enrich.hostname: {}", config.enrich.hostname);
        println!("  taxonomy.auto_classify_enabled: {}", config.taxonomy.auto_classify_enabled);
        println!(
            "  crypto.key_env: {}",
            config.crypto.key_env.as_deref().unwrap_or("(off)")
        );
    };

    // "config encrypt" always jumps straight to the encryption flow, no menu.
    // No action + a real terminal gets the arrow-key menu. Anything else
    // (no action, non-interactive — the pre-existing behavior, e.g. in CI)
    // just prints the resolved config, unchanged.
    if action.as_deref() == Some("encrypt") {
        return run_config_encryption_flow(&db_path, &config);
    }

    if action.is_none() && is_interactive_terminal() {
        return run_config_menu(&db_path, &config, print_resolved);
    }

    print_resolved();
    Ok(())
}

fn run_config_menu(
    db_path: &std::path::Path,
    config: &Config,
    print_resolved: impl Fn(),
) -> Result<(), Box<dyn std::error::Error>> {
    use dialoguer::{theme::ColorfulTheme, Select};

    let items = [
        "Show current settings",
        "Turn on / change encryption for sensitive data",
        "Toggle buffered (async) writes — buffer.enabled",
        "Toggle hostname enrichment — enrich.hostname",
        "Toggle automatic error classification — taxonomy.auto_classify_enabled",
        "Exit without changes",
    ];

    let choice = match Select::with_theme(&ColorfulTheme::default())
        .with_prompt("What would you like to do? (↑/↓ to move, enter to select)")
        .items(&items)
        .default(0)
        .interact_opt()
    {
        Ok(Some(choice)) => choice,
        Ok(None) | Err(_) => return Ok(()), // Ctrl+C / cancelled
    };

    match choice {
        0 => print_resolved(),
        1 => run_config_encryption_flow(db_path, config)?,
        2 => toggle_bool_setting(
            db_path,
            "buffer",
            "enabled",
            config.buffer.enabled,
            "Buffer writes asynchronously in a background queue instead of blocking the caller on every write (trades a small durability window for lower write latency)",
        )?,
        3 => toggle_bool_setting(
            db_path,
            "enrich",
            "hostname",
            config.enrich.hostname,
            "Attach the machine's hostname to every logged event's context (useful for multi-host deployments, off if hostnames are sensitive)",
        )?,
        4 => toggle_bool_setting(
            db_path,
            "taxonomy",
            "auto_classify_enabled",
            config.taxonomy.auto_classify_enabled,
            "Automatically assign a category to each event via the builtin rules engine when none is set explicitly",
        )?,
        _ => {}
    }

    Ok(())
}

/// Generic on/off toggle for a `[section] key = bool` config.toml entry,
/// shared by every boolean item in `run_config_menu`. Mirrors
/// `enable_metadata_encryption`'s use of `toml_edit` so a pre-existing
/// config.toml's other sections/comments/formatting are left untouched.
fn toggle_bool_setting(
    db_path: &std::path::Path,
    section: &str,
    key: &str,
    current: bool,
    description: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use dialoguer::{theme::ColorfulTheme, Confirm};

    println!();
    println!("{description}");
    let new_value = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(format!("{section}.{key} (currently {current}) — turn on?"))
        .default(current)
        .interact()
        .unwrap_or(current);

    if new_value == current {
        println!("No changes made.");
        return Ok(());
    }

    let witslog_dir = db_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".witslog"));
    let config_path = witslog_dir.join("config.toml");

    let mut doc: toml_edit::DocumentMut = if config_path.exists() {
        std::fs::read_to_string(&config_path)?.parse()?
    } else {
        toml_edit::DocumentMut::new()
    };
    if doc.get(section).and_then(|i| i.as_table()).is_none() {
        doc[section] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    doc[section][key] = toml_edit::value(new_value);
    std::fs::write(&config_path, doc.to_string())?;

    println!("✓ {section}.{key} set to {new_value} (config.toml)");
    Ok(())
}

/// Shared by `config encrypt` (explicit) and the interactive menu's
/// "Turn on / change encryption" item. If encryption is already on, offers to
/// rotate (generate + store a brand-new key under the same or a renamed env
/// var) rather than silently clobbering the existing setup.
fn run_config_encryption_flow(
    db_path: &std::path::Path,
    config: &Config,
) -> Result<(), Box<dyn std::error::Error>> {
    let witslog_dir = db_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".witslog"));

    if let Some(existing_var) = &config.crypto.key_env {
        use dialoguer::{theme::ColorfulTheme, Confirm};
        println!("Encryption is already on (env var: {existing_var}).");
        let rotate = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt(
                "Generate a brand-new key now? (rows encrypted under the old key stay \
                 readable only if you still have that old key saved somewhere)",
            )
            .default(false)
            .interact()
            .unwrap_or(false);

        if !rotate {
            println!("No changes made.");
            return Ok(());
        }
        enable_metadata_encryption(&witslog_dir, existing_var)?;
        return Ok(());
    }

    if let Some(var_name) = run_init_encryption_prompt()? {
        enable_metadata_encryption(&witslog_dir, &var_name)?;
    } else {
        println!("No changes made.");
    }

    Ok(())
}

fn cmd_archive(
    db_override: &Option<PathBuf>,
    older_than: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir()?;
    let config = Config::default_project();
    let db_path = db_override.clone().unwrap_or_else(|| config.resolve_db_path(&cwd));

    let older_than = match older_than {
        Some(s) => s,
        None => {
            eprintln!("Error: --older-than is required (e.g. --older-than 90d)");
            std::process::exit(2);
        }
    };

    let age_ms = match parse_age_ms(&older_than) {
        Some(ms) => ms,
        None => {
            eprintln!("Error: invalid --older-than value '{}' (expected e.g. 90d, 24h)", older_than);
            std::process::exit(2);
        }
    };

    let cutoff_ms = chrono::Utc::now().timestamp_millis() - age_ms;

    let store = Store::open_or_create(&db_path)?;
    let conn = store.conn().conn();

    let now = chrono::Utc::now().format("%Y-Q%m").to_string();
    let archive_path = db_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(format!("witslog-{}.db", now));

    // Attach archive DB and ensure it has the events table (idempotent schema copy).
    conn.execute(
        &format!("ATTACH DATABASE '{}' AS archive", archive_path.display().to_string().replace('\'', "''")),
        [],
    )?;

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS archive.events AS SELECT * FROM events WHERE 0;",
    )?;

    let moved: usize = conn.execute(
        "INSERT INTO archive.events SELECT * FROM events WHERE ts_epoch_ms < ?1",
        [cutoff_ms],
    )?;

    let ids: Vec<String> = {
        let mut stmt = conn.prepare("SELECT event_id FROM events WHERE ts_epoch_ms < ?1")?;
        let rows = stmt.query_map([cutoff_ms], |row| row.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    conn.execute("DETACH DATABASE archive", [])?;
    drop(conn);

    // Same tombstone-then-delete path as `prune`/`delete` (FR-P10-001).
    let writer = witslog_store::EventWriter::new(store.conn());
    writer.delete_by_ids(&ids)?;

    println!("✓ Archived {} event(s) older than {} to {}", moved, older_than, archive_path.display());

    Ok(())
}

fn cmd_backup(
    db_override: &Option<PathBuf>,
    output: &PathBuf,
    force: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir()?;
    let config = Config::default_project();
    let db_path = db_override.clone().unwrap_or_else(|| config.resolve_db_path(&cwd));

    if output.exists() && !force {
        eprintln!(
            "Error: backup target '{}' already exists (use --force to overwrite)",
            output.display()
        );
        std::process::exit(2);
    }

    let store = Store::open_or_create(&db_path)?;
    let conn = store.conn().conn();

    // Simple backup: checkpoint WAL then copy file.
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE);", [], |_| Ok(()))?;
    std::fs::copy(&db_path, output)?;

    println!("✓ Backup created: {}", output.display());

    Ok(())
}

fn cmd_category(
    db_override: &Option<PathBuf>,
    action: CategoryAction,
) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir()?;
    let config = Config::default_project();
    let db_path = db_override.clone().unwrap_or_else(|| config.resolve_db_path(&cwd));

    let store = Store::open_or_create(&db_path)?;
    let conn = store.conn().conn();

    match action {
        CategoryAction::Add { canonical, parent, label } => {
            let label = label.unwrap_or_else(|| canonical.clone());
            match witslog_store::taxonomy::insert_category(&conn, &canonical, parent.as_deref(), &label, false) {
                Ok(()) => {
                    println!("✓ Category added: {}", canonical);
                }
                Err(witslog_store::StoreError::CategoryCollision(c)) => {
                    eprintln!("Error: '{}' collides with an existing builtin category", c);
                    std::process::exit(2);
                }
                Err(e) => return Err(e.into()),
            }
        }
        CategoryAction::Alias { alias, canonical } => {
            match witslog_store::taxonomy::insert_alias(&conn, &alias, &canonical) {
                Ok(()) => {
                    println!("✓ Alias registered: {} -> {}", alias, canonical);
                }
                Err(witslog_store::StoreError::UnknownCanonical(c)) => {
                    eprintln!("Error: alias targets unknown canonical '{}'", c);
                    std::process::exit(2);
                }
                Err(e) => return Err(e.into()),
            }
        }
        CategoryAction::List => {
            let categories = witslog_store::taxonomy::list_categories(&conn)?;
            println!("Categories ({}):", categories.len());
            for (canonical, label, parent) in categories {
                match parent {
                    Some(p) => println!("  {} ({}) [parent: {}]", canonical, label, p),
                    None => println!("  {} ({})", canonical, label),
                }
            }
        }
    }

    Ok(())
}

fn read_schema_version(db_path: &PathBuf) -> Option<i32> {
    if !db_path.exists() {
        return None;
    }
    let conn = rusqlite::Connection::open(db_path).ok()?;
    conn.query_row(
        "SELECT COALESCE(value, '0') FROM schema_meta WHERE key = 'schema_version'",
        [],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .and_then(|s| s.parse().ok())
}

fn cmd_migrate(db_override: &Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir()?;
    let config = Config::default_project();
    let db_path = db_override.clone().unwrap_or_else(|| config.resolve_db_path(&cwd));

    if !db_path.exists() {
        return Err(format!(
            "Database not initialized. Run 'witslog init' in {}",
            cwd.display()
        )
        .into());
    }

    let before = read_schema_version(&db_path).unwrap_or(0);

    let backup_path = db_path.with_extension("bak");
    std::fs::copy(&db_path, &backup_path)?;

    // Store::open_or_create runs pending migrations on open. On failure
    // (including the version-compat guard rejecting a too-new DB), restore
    // the pre-migration snapshot rather than leaving a half-migrated file.
    let store_result = Store::open_or_create(&db_path);
    if let Err(e) = store_result {
        std::fs::copy(&backup_path, &db_path)?;
        std::fs::remove_file(&backup_path).ok();
        return Err(format!("migration failed, restored backup: {}", e).into());
    }

    let after = read_schema_version(&db_path).unwrap_or(0);

    println!("✓ Migration complete");
    println!("  schema version: {} -> {}", before, after);
    println!("  backup: {}", backup_path.display());

    Ok(())
}

fn cmd_list_dbs() -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir()?;

    println!("Known project DBs:");
    let mut found = false;

    // Walk up from cwd looking for .witslog/ dirs.
    let mut current = cwd.clone();
    loop {
        let marker = current.join(".witslog");
        if marker.exists() {
            let db = marker.join("witslog.db");
            if db.exists() {
                println!("  {}", db.display());
                found = true;
            }
        }

        if !current.pop() {
            break;
        }
    }

    if !found {
        println!("  (none found)");
    }

    Ok(())
}

fn cmd_serve_mcp(
    db_override: &Option<PathBuf>,
    attach: Vec<PathBuf>,
    allow_write: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir()?;
    // `load_or_default` (not `default_project`) so `[crypto] key_env` (and
    // any other `.witslog/config.toml` section) is actually respected here —
    // without this, the MCP server could never know which env var holds the
    // metadata-decryption key regardless of what the project's config says.
    let config = Config::load_or_default(&cwd);
    let db_path = db_override.clone().unwrap_or_else(|| config.resolve_db_path(&cwd));

    if !db_path.exists() {
        return Err(format!(
            "no witslog DB found at {} (run `witslog init` first)",
            db_path.display()
        )
        .into());
    }

    // Read-only by construction unless --allow-write (FR-P5-005). The write
    // path (witslog_delete) needs a writable handle, so open read-write only
    // when explicitly requested.
    let db = if allow_write {
        witslog_store::DbConnection::open(&db_path)?
    } else {
        witslog_store::DbConnection::open_read_only(&db_path)?
    };

    let mcp_config = witslog_mcp::ServerConfig {
        allow_write,
        attached: attach,
        statement_timeout: witslog_mcp::server::DEFAULT_STATEMENT_TIMEOUT,
        crypto_key_env: config.crypto.key_env.clone(),
    };

    witslog_mcp::serve_stdio(&db, mcp_config)?;

    Ok(())
}
