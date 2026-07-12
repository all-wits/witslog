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
        let classifier = Classifier::built_in();
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
