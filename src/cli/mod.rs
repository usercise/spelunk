use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

pub mod commands;

/// ca — local code search and understanding
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Path to config file (default: ~/.config/codeanalysis/config.toml)
    #[arg(short, long, global = true)]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Index a codebase directory
    Index(IndexArgs),
    /// Semantic search over the index
    Search(SearchArgs),
    /// Ask a natural language question (full RAG pipeline)
    Ask(AskArgs),
    /// Show index statistics
    Status,
    /// List supported languages
    Languages,
    /// Query the code graph (imports, calls, extends/implements)
    Graph(GraphArgs),
}

#[derive(Args, Debug)]
pub struct IndexArgs {
    /// Path to the codebase root to index
    pub path: PathBuf,

    /// Path to the SQLite database (overrides config)
    #[arg(short, long)]
    pub db: Option<PathBuf>,

    /// Embedding batch size
    #[arg(long, default_value = "32")]
    pub batch_size: usize,

    /// Force full re-index (ignore change detection)
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct SearchArgs {
    /// Natural language search query
    pub query: String,

    /// Number of results to return
    #[arg(short, long, default_value = "10")]
    pub limit: usize,

    /// Output format: text or json
    #[arg(long, default_value = "text")]
    pub format: String,

    /// Path to the SQLite database (overrides config)
    #[arg(short, long)]
    pub db: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct AskArgs {
    /// Question to answer using the indexed codebase
    pub question: String,

    /// Number of chunks to retrieve as context
    #[arg(long, default_value = "10")]
    pub context_chunks: usize,

    /// Path to the SQLite database (overrides config)
    #[arg(short, long)]
    pub db: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct GraphArgs {
    /// Symbol name or file path to look up in the graph
    pub symbol: String,

    /// Filter to a specific edge kind: imports, calls, extends, implements
    #[arg(long)]
    pub kind: Option<String>,

    /// Output format: text or json
    #[arg(long, default_value = "text")]
    pub format: String,

    /// Path to the SQLite database (overrides config)
    #[arg(short, long)]
    pub db: Option<PathBuf>,
}
