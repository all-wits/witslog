use clap::{Parser, Subcommand};
use std::path::PathBuf;
use witslog_config::Config;
use witslog_core::{EventBuilder, Severity};
use witslog_store::Store;

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
    },
    Query {
        event_id: String,
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
        } => {
            log_event(&cli.db, &app, &message, version, environment, severity, category)?;
        }
        Commands::Query { event_id } => {
            query_event(&cli.db, &event_id)?;
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
) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir()?;

    let config = Config::default_project();
    let db_path = db_override.clone().unwrap_or_else(|| config.resolve_db_path(&cwd));

    if !db_path.parent().and_then(|p| Some(p.exists())).unwrap_or(false) {
        return Err(format!(
            "Database not initialized. Run 'witslog init' in {}",
            cwd.display()
        )
        .into());
    }

    let store = Store::open_or_create(&db_path)?;

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

    let event = builder.build();

    let writer = witslog_store::EventWriter::new(store.conn());
    let _row_id = writer.write(&event)?;

    println!("✓ Event logged");
    println!("  event_id: {}", event.event_id);
    println!("  fingerprint: {}", event.fingerprint);
    println!("  DB: {}", db_path.display());

    Ok(())
}

fn query_event(db_override: &Option<PathBuf>, event_id: &str) -> Result<(), Box<dyn std::error::Error>> {
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
        }
        None => {
            println!("Event not found: {}", event_id);
        }
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
            let _ = store;
        }
    }

    Ok(())
}
