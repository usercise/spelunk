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
    /// Show index statistics (for current project or all registered projects)
    Status(StatusArgs),
    /// List supported languages
    Languages,
    /// Query the code graph (imports, calls, extends/implements)
    Graph(GraphArgs),
    /// Show the raw indexed chunks for a file (useful for debugging/agent use)
    Chunks(ChunksArgs),
    /// Add a dependency: current project also searches another project's index
    Link(LinkArgs),
    /// Remove a previously added dependency
    Unlink(UnlinkArgs),
    /// Remove registry entries for projects whose root path no longer exists
    Autoclean,
    /// Project memory: store and query decisions, context, and requirements
    Memory(MemoryArgs),
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

    /// Number of results to return (max 100)
    #[arg(short, long, default_value = "10")]
    pub limit: usize,

    /// Output format: text or json
    #[arg(long, default_value = "text")]
    pub format: String,

    /// Enrich results with 1-hop call-graph neighbours (callers + callees)
    #[arg(short, long)]
    pub graph: bool,

    /// Maximum number of graph-expanded results to add (when --graph is set)
    #[arg(long, default_value = "10")]
    pub graph_limit: usize,

    /// Path to the SQLite database (overrides config)
    #[arg(short, long)]
    pub db: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct AskArgs {
    /// Question to answer using the indexed codebase
    pub question: String,

    /// Number of chunks to retrieve as context (max 100)
    #[arg(long, default_value = "20")]
    pub context_chunks: usize,

    /// Return structured JSON: { answer, relevant_files, confidence }
    #[arg(long)]
    pub json: bool,

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

#[derive(Args, Debug)]
pub struct ChunksArgs {
    /// File path (exact or suffix match against indexed paths)
    pub path: String,

    /// Output format: text or json
    #[arg(long, default_value = "text")]
    pub format: String,

    /// Path to the SQLite database (overrides config)
    #[arg(short, long)]
    pub db: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct StatusArgs {
    /// Show stats for all registered projects, not just the current one
    #[arg(short, long)]
    pub all: bool,

    /// Brief list format (one line per project) — implies --all
    #[arg(short, long)]
    pub list: bool,
}

#[derive(Args, Debug)]
pub struct LinkArgs {
    /// Path to the project to add as a dependency
    pub path: PathBuf,

    /// Path to the SQLite database for the current project (overrides auto-detect)
    #[arg(short, long)]
    pub db: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct UnlinkArgs {
    /// Path to the project to remove as a dependency
    pub path: PathBuf,

    /// Path to the SQLite database for the current project (overrides auto-detect)
    #[arg(short, long)]
    pub db: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct MemoryArgs {
    #[command(subcommand)]
    pub command: MemoryCommand,

    /// Path to the memory database (overrides auto-detect)
    #[arg(long, global = true)]
    pub db: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
pub enum MemoryCommand {
    /// Store a decision, context, requirement, or note
    Add(MemoryAddArgs),
    /// Semantic search over stored memory
    Search(MemorySearchArgs),
    /// List memory entries (newest first)
    List(MemoryListArgs),
    /// Show the full content of a memory entry
    Show(MemoryShowArgs),
}

#[derive(Args, Debug)]
pub struct MemoryAddArgs {
    /// Short title summarising the entry
    #[arg(short, long)]
    pub title: String,

    /// Full body text
    #[arg(short, long)]
    pub body: String,

    /// Kind: decision, context, requirement, note
    #[arg(short, long, default_value = "note")]
    pub kind: String,

    /// Comma-separated tags (e.g. auth,database)
    #[arg(long)]
    pub tags: Option<String>,

    /// Comma-separated file paths this entry relates to
    #[arg(long)]
    pub files: Option<String>,
}

#[derive(Args, Debug)]
pub struct MemorySearchArgs {
    /// Natural language query
    pub query: String,

    /// Number of results to return
    #[arg(short, long, default_value = "10")]
    pub limit: usize,

    /// Output format: text or json
    #[arg(long, default_value = "text")]
    pub format: String,
}

#[derive(Args, Debug)]
pub struct MemoryListArgs {
    /// Filter by kind: decision, context, requirement, note
    #[arg(short, long)]
    pub kind: Option<String>,

    /// Number of entries to show
    #[arg(short, long, default_value = "20")]
    pub limit: usize,

    /// Output format: text or json
    #[arg(long, default_value = "text")]
    pub format: String,
}

#[derive(Args, Debug)]
pub struct MemoryShowArgs {
    /// Entry ID (from list or search output)
    pub id: i64,

    /// Output format: text or json
    #[arg(long, default_value = "text")]
    pub format: String,
}
