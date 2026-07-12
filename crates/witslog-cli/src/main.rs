use clap::{Parser, Subcommand};
use std::path::PathBuf;
use witslog_config::Config;
use witslog_core::{AsyncBuffer, BufferConfig, Classifier, EnrichConfig, EventBuilder, Redactor, Severity};
use witslog_store::{DeleteFilter, Store, StoreSink};

#[derive(Parser)]
#[command(name = "witslog")]
#[command(about = "AI-native error logging framework", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    #[arg(global = true, short, long)]
    db: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    Init {
        #[arg(value_name = "PATH", default_value = ".")]
        path: PathBuf,
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
    Doctor,
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
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Init { path } => {
            init_db(&path)?;
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
            get_event(&cli.db, &event_id)?;
        }
        Commands::Query {
            text,
            application,
            category,
            severity_min,
            limit,
            cursor,
        } => {
            query_search(&cli.db, &text, application, category, severity_min, limit, cursor)?;
        }
        Commands::Stats { application, severity_min } => {
            cmd_stats(&cli.db, application, severity_min)?;
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
        Commands::Resolve { event_id } => {
            resolve_event(&cli.db, &event_id)?;
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
        Commands::Doctor => {
            doctor()?;
        }
        Commands::Category { action } => {
            cmd_category(&cli.db, action)?;
        }
        Commands::ServeMcp {
            stdio: _,
            attach,
            allow_write,
        } => {
            cmd_serve_mcp(&cli.db, attach, allow_write)?;
        }
    }

    Ok(())
}

fn init_db(path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
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

    println!("✓ Initialized witslog at {}", db_path.display());
    println!("  DB path: {}", db_path.display());

    Ok(())
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

    if !db_path.parent().and_then(|p| Some(p.exists())).unwrap_or(false) {
        return Err(format!(
            "Database not initialized. Run 'witslog init' in {}",
            cwd.display()
        )
        .into());
    }

    let store = Store::open_or_create(&db_path)?;

    let redactor = match Redactor::new(&config.redact.custom_patterns) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Invalid redaction pattern in config: {e}");
            std::process::exit(2);
        }
    };
    let enrich_cfg = EnrichConfig {
        hostname: config.enrich.hostname,
        pid: config.enrich.pid,
        cwd: config.enrich.cwd,
        argv: config.enrich.argv,
        git_commit: config.enrich.git_commit,
        env_allowlist: config.enrich.env_allowlist.clone(),
    };

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

    let mut builder = EventBuilder::new(app, message)
        .severity(severity);

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

    builder = builder.enrich(&enrich_cfg).redact(&redactor);

    if config.taxonomy.auto_classify_enabled {
        let classifier = match &config.taxonomy.custom_rules_file {
            Some(path) => match witslog_core::load_custom_rules(path) {
                Ok(rules) => Classifier::built_in_with_custom(rules),
                Err(e) => {
                    eprintln!("Invalid custom rules file {}: {e}", path.display());
                    std::process::exit(2);
                }
            },
            None => Classifier::built_in(),
        };
        builder = builder.classify(&classifier);
    }

    let event = builder.build();

    if config.buffer.enabled {
        let buffer_cfg = BufferConfig {
            enabled: config.buffer.enabled,
            batch_size: config.buffer.batch_size,
            flush_interval_ms: config.buffer.flush_interval_ms,
            queue_capacity: config.buffer.queue_capacity,
        };
        let sink = StoreSink::new(store);
        let buffer = AsyncBuffer::new(sink, buffer_cfg);
        buffer.enqueue(event.clone());
        // Dropping joins the flush thread, guaranteeing the event is
        // persisted (or counted as dropped) before this short-lived
        // process exits.
        drop(buffer);
    } else {
        let writer = witslog_store::EventWriter::new(store.conn());
        let _row_id = writer.write(&event)?;
    }

    println!("✓ Event logged");
    println!("  event_id: {}", event.event_id);
    println!("  fingerprint: {}", event.fingerprint);
    println!("  DB: {}", db_path.display());

    Ok(())
}

fn get_event(db_override: &Option<PathBuf>, event_id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir()?;

    let config = Config::default_project();
    let db_path = db_override.clone().unwrap_or_else(|| config.resolve_db_path(&cwd));

    let store = Store::open_or_create(&db_path)?;
    let writer = witslog_store::EventWriter::new(store.conn());

    match writer.query_by_id(event_id)? {
        Some(event) => {
            println!("Event found:");
            println!("  event_id: {}", event.event_id);
            println!("  timestamp: {}", event.timestamp);
            println!("  application: {}", event.application);
            println!("  message: {}", event.message);
            println!("  severity: {}", event.severity.as_str());
            println!("  fingerprint: {}", event.fingerprint);
            if let Some(cat) = &event.category {
                println!("  category: {}", cat);
            }
            match &event.resolved_at {
                Some(r) => println!("  resolved_at: {}", r),
                None => println!("  resolved_at: (unresolved)"),
            }
        }
        None => {
            println!("Event not found: {}", event_id);
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
    limit: usize,
    cursor: Option<String>,
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
        ..Default::default()
    };

    let result = search.search(text, &filters, limit, cursor, true)?;

    if let Some(warning) = &result.cursor_warning {
        eprintln!("warning: {}", warning);
    }

    if result.items.is_empty() {
        println!("No matching events.");
    }
    for event in &result.items {
        println!("{}  [{}] {:?} :: {}", event.event_id, event.application, event.severity, event.message);
    }
    println!("\n{} match(es) (showing {})", result.total_estimate, result.items.len());
    if let Some(next) = result.next_cursor {
        println!("next cursor: {}", next);
    }

    Ok(())
}

fn resolve_event(db_override: &Option<PathBuf>, event_id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir()?;

    let config = Config::default_project();
    let db_path = db_override.clone().unwrap_or_else(|| config.resolve_db_path(&cwd));

    let store = Store::open_or_create(&db_path)?;
    let writer = witslog_store::EventWriter::new(store.conn());
    writer.mark_resolved(event_id)?;

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
        println!("(dry run — no rows deleted; re-run without --dry-run to apply)");
        // dry_run preview reuses the same filter but never mutates: delete_resolved
        // itself performs the delete, so a true dry-run just prints the intended filter.
        println!("  filter: {:?}", filter);
        return Ok(());
    }

    let deleted_ids = writer.delete_resolved(&filter)?;

    println!("✓ Deleted {} event(s)", deleted_ids.len());
    for id in &deleted_ids {
        println!("  {}", id);
    }

    Ok(())
}

fn doctor() -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir()?;
    let config = Config::default_project();
    let db_path = config.resolve_db_path(&cwd);

    println!("witslog doctor");
    println!("  cwd: {}", cwd.display());
    println!("  resolved db: {}", db_path.display());
    println!("  db exists: {}", db_path.exists());

    if db_path.exists() {
        if let Ok(store) = Store::open_or_create(&db_path) {
            println!("  ✓ database healthy");
            let writer = witslog_store::EventWriter::new(store.conn());
            if let Ok(dropped) = writer.dropped_count() {
                println!("  dropped events (lifetime): {}", dropped);
            }
        }
    }

    Ok(())
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

    for id in &ids {
        conn.execute("DELETE FROM events WHERE event_id = ?1", [id])?;
        conn.execute(
            "DELETE FROM error_edges WHERE src_event_id = ?1 OR dst_event_id = ?1",
            [id],
        )?;
    }

    println!("✓ Pruned {} event(s) older than {}", ids.len(), older_than);

    Ok(())
}

fn cmd_config(
    db_override: &Option<PathBuf>,
    _action: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir()?;
    let config = Config::load_or_default(&cwd);
    let db_path = db_override.clone().unwrap_or_else(|| config.resolve_db_path(&cwd));

    println!("Config resolved:");
    println!("  db path: {}", db_path.display());
    println!("  buffer.enabled: {}", config.buffer.enabled);
    println!("  enrich.hostname: {}", config.enrich.hostname);
    println!("  taxonomy.auto_classify_enabled: {}", config.taxonomy.auto_classify_enabled);

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

    conn.execute("DELETE FROM events WHERE ts_epoch_ms < ?1", [cutoff_ms])?;
    conn.execute("DETACH DATABASE archive", [])?;

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

    // Store::open_or_create runs pending migrations on open.
    let _store = Store::open_or_create(&db_path)?;

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
    let config = Config::default_project();
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
    };

    witslog_mcp::serve_stdio(&db, mcp_config)?;

    Ok(())
}
