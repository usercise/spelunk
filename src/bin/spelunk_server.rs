use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use clap::Parser;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

// Pull in the spelunk library crate (same workspace).
use spelunk::server::db::ServerDb;
use spelunk::server::{AppState, router};

#[derive(Parser, Debug)]
#[command(name = "spelunk-server", about = "Shared memory server for spelunk")]
struct Args {
    /// Port to listen on
    #[arg(long, default_value = "7777")]
    port: u16,

    /// Host/address to bind
    #[arg(long, default_value = "0.0.0.0")]
    host: String,

    /// Path to the server SQLite database
    #[arg(long, default_value = "spelunk.db")]
    db: PathBuf,

    /// Shared API key (Bearer token). Leave unset to disable auth (dev only).
    #[arg(long, env = "SPELUNK_SERVER_KEY")]
    key: Option<String>,

    /// Embedding dimension expected from clients (must match the team's model).
    /// Default: 768 (EmbeddingGemma 300M).
    #[arg(long, default_value = "768")]
    embedding_dim: usize,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Register sqlite-vec extension for every connection in this process.
    #[allow(clippy::missing_transmute_annotations)]
    unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    }

    // Logging.
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with(fmt::layer())
        .init();

    let args = Args::parse();

    let db = ServerDb::open(&args.db, args.embedding_dim)
        .with_context(|| format!("opening server db at {}", args.db.display()))?;

    if args.key.is_none() {
        tracing::warn!(
            "No API key configured — server is running without authentication. Set --key or SPELUNK_SERVER_KEY for production use."
        );
    }

    let state = AppState {
        db: Arc::new(tokio::sync::Mutex::new(db)),
        api_key: args.key,
    };

    let app = router(state);
    let addr: SocketAddr = format!("{}:{}", args.host, args.port)
        .parse()
        .context("parsing bind address")?;

    tracing::info!("spelunk-server listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
