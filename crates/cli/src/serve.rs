//! Server bootstrap: validated configuration, logging, migrations, graceful shutdown.

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, bail};
use clap::{ArgAction, Args, ValueEnum};
use specsentry_http::{AppState, HttpConfig, router, service};
use specsentry_store::Store;
use tracing_subscriber::EnvFilter;

const MIN_BODY_BYTES: usize = 1024;
const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;

/// Every option can also be set through the environment variable shown.
#[derive(Debug, Args)]
pub struct ServeArgs {
    /// Interface to bind. Use 0.0.0.0 inside containers.
    #[arg(long = "bind-addr", env = "BIND_ADDR", default_value = "127.0.0.1")]
    bind_addr: IpAddr,
    /// TCP port.
    #[arg(long, env = "PORT", default_value_t = 3005)]
    port: u16,
    /// SQLite database file (created with its directory if missing).
    #[arg(long, env = "DATABASE_PATH", default_value = "data/specsentry.sqlite")]
    database_path: PathBuf,
    /// Directory with the static web UI.
    #[arg(long, env = "PUBLIC_DIR", default_value = "public")]
    public_dir: PathBuf,
    /// Maximum request body size in bytes (1 KiB to 64 MiB).
    #[arg(long, env = "MAX_BODY_BYTES", default_value_t = 2 * 1024 * 1024, value_parser = parse_body_limit)]
    max_body_bytes: usize,
    /// Per-request timeout in seconds (1 to 300).
    #[arg(long, env = "REQUEST_TIMEOUT_SECS", default_value_t = 15, value_parser = clap::value_parser!(u64).range(1..=300))]
    request_timeout_secs: u64,
    /// Log output format.
    #[arg(long, env = "LOG_FORMAT", value_enum, default_value_t = LogFormat::Json)]
    log_format: LogFormat,
    /// Seed a demo project when the database is empty.
    #[arg(long, env = "SEED_DEMO", default_value_t = true, action = ArgAction::Set)]
    seed_demo: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum LogFormat {
    Json,
    Pretty,
}

fn parse_body_limit(raw: &str) -> Result<usize, String> {
    let value: usize = raw
        .parse()
        .map_err(|_| format!("`{raw}` is not a number"))?;
    if (MIN_BODY_BYTES..=MAX_BODY_BYTES).contains(&value) {
        Ok(value)
    } else {
        Err(format!(
            "must be between {MIN_BODY_BYTES} and {MAX_BODY_BYTES} bytes"
        ))
    }
}

pub fn run(args: ServeArgs) -> anyhow::Result<()> {
    if !args.public_dir.join("index.html").is_file() {
        bail!(
            "PUBLIC_DIR {} does not contain index.html",
            args.public_dir.display()
        );
    }
    init_logging(args.log_format);

    let store = Store::open(&args.database_path)
        .with_context(|| format!("opening database {}", args.database_path.display()))?;
    tracing::info!(
        path = %args.database_path.display(),
        schema_version = store.schema_version()?,
        "database ready"
    );
    if args.seed_demo && service::seed_demo(&store).map_err(|e| anyhow::anyhow!(e.message))? {
        tracing::info!("seeded demo project");
    }

    let config = HttpConfig {
        public_dir: args.public_dir,
        max_body_bytes: args.max_body_bytes,
        request_timeout: Duration::from_secs(args.request_timeout_secs),
    };
    let app = router(AppState::new(store), &config);
    let address = SocketAddr::new(args.bind_addr, args.port);

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("starting async runtime")?
        .block_on(async move {
            let listener = tokio::net::TcpListener::bind(address)
                .await
                .with_context(|| format!("binding {address}"))?;
            tracing::info!(address = %listener.local_addr()?, "SpecSentry listening");
            axum::serve(listener, app)
                .with_graceful_shutdown(shutdown_signal())
                .await
                .context("server error")?;
            tracing::info!("shutdown complete");
            Ok(())
        })
}

fn init_logging(format: LogFormat) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    match format {
        LogFormat::Json => builder
            .json()
            .flatten_event(true)
            .with_current_span(true)
            .with_span_list(false)
            .init(),
        LogFormat::Pretty => builder.init(),
    }
}

/// Resolves on Ctrl+C or SIGTERM (what `docker stop` sends).
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::error!(%err, "cannot listen for Ctrl+C");
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(err) => {
                tracing::error!(%err, "cannot listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutdown signal received, draining connections");
}
