use anyhow::Result;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

mod cli;

use clap::{CommandFactory, FromArgMatches};
use cli::{Cli, Command};
use spelunk::{backends, config, embeddings, indexer, llm, registry, search, storage, utils};

#[tokio::main]
async fn main() -> Result<()> {
    // Register sqlite-vec for every SQLite connection opened in this process.
    // SAFETY: sqlite3_auto_extension stores the pointer and SQLite calls it
    // with the correct (db, err_msg, api) arguments at connection time.
    #[allow(clippy::missing_transmute_annotations)]
    unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    }

    // Logging: RUST_LOG=debug spelunk ...
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    // Pre-check: does the config have llm_model set?
    // Scan args for --config/-c to find the right config file before full parse.
    let pre_config_path = {
        let args: Vec<String> = std::env::args().collect();
        args.windows(2)
            .find(|w| w[0] == "--config" || w[0] == "-c")
            .map(|w| std::path::PathBuf::from(&w[1]))
    };
    let llm_configured = config::Config::load(pre_config_path.as_deref())
        .map(|c| c.llm_model.is_some())
        .unwrap_or(false);

    // Hide `ask` from help when no chat model is configured.
    let matches = Cli::command()
        .mut_subcommand("ask", |c| c.hide(!llm_configured))
        .get_matches();
    let cli = Cli::from_arg_matches(&matches)?;

    let cfg = config::Config::load(cli.config.as_deref())?;

    match cli.command {
        Command::Init(args) => cli::cmd::init(args, cfg).await,
        Command::Index(args) => cli::cmd::index(args, cfg).await,
        Command::Search(args) => cli::cmd::search(args, cfg).await,
        Command::Ask(args) => cli::cmd::ask(args, cfg).await,
        Command::Status(args) => cli::cmd::status(args, cfg).await,
        Command::Check(args) => cli::cmd::check(args, cfg),
        Command::Languages => cli::cmd::languages(),
        Command::Graph(args) => cli::cmd::graph(args, cfg),
        Command::Chunks(args) => cli::cmd::chunks(args, cfg),
        Command::Verify(args) => cli::cmd::verify(args, cfg).await,
        Command::Link(args) => cli::cmd::link(args, cfg),
        Command::Unlink(args) => cli::cmd::unlink(args, cfg),
        Command::Autoclean => cli::cmd::autoclean(cfg),
        Command::Memory(args) => cli::cmd::memory(args, cfg).await,
        Command::Hooks(args) => cli::cmd::hooks(args),
        Command::Plan(args) => cli::cmd::plan(args, cfg).await,
        Command::Spec(args) => cli::cmd::spec(args, cfg),
        Command::Links(args) => cli::cmd::links(args, cfg).await,
    }
}
