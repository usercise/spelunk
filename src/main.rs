use anyhow::Result;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

mod backends;
mod cli;
mod config;
mod embeddings;
mod error;
mod indexer;
mod llm;
mod search;
mod storage;

use cli::{Cli, Command};
use clap::Parser;

#[tokio::main]
async fn main() -> Result<()> {
    // Logging: RUST_LOG=debug ca ...
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let cfg = config::Config::load(cli.config.as_deref())?;

    match cli.command {
        Command::Index(args) => cli::commands::index(args, cfg).await,
        Command::Search(args) => cli::commands::search(args, cfg).await,
        Command::Ask(args) => cli::commands::ask(args, cfg).await,
        Command::Status => cli::commands::status(cfg).await,
        Command::Languages => cli::commands::languages(),
    }
}
