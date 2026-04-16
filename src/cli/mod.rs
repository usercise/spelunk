use clap::{Parser, Subcommand};

mod args;
pub mod cmd;

pub use args::*;

/// spelunk — local code intelligence
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Path to config file (default: ~/.config/spelunk/config.toml)
    #[arg(short, long, global = true)]
    pub config: Option<std::path::PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Initialise spelunk for the current project
    Init(InitArgs),
    /// Index a codebase directory
    Index(IndexArgs),
    /// Semantic search over the index
    Search(SearchArgs),
    /// Ask a natural language question (full RAG pipeline)
    Ask(AskArgs),
    /// Show index statistics (for current project or all registered projects)
    Status(StatusArgs),
    /// Check whether the index is in sync with the current source tree (exit 1 if stale)
    Check(CheckArgs),
    /// List supported languages
    Languages,
    /// Query the code graph (imports, calls, extends/implements)
    Graph(GraphArgs),
    /// Show the raw indexed chunks for a file (useful for debugging/agent use)
    Chunks(ChunksArgs),
    /// Verify semantic coherence of a file or symbol after changes
    Verify(VerifyArgs),
    /// Add a dependency: current project also searches another project's index
    Link(LinkArgs),
    /// Remove a previously added dependency
    Unlink(UnlinkArgs),
    /// Remove registry entries for projects whose root path no longer exists
    Autoclean,
    /// Project memory: store and query decisions, context, and requirements
    Memory(MemoryArgs),
    /// Manage git hooks (post-commit auto-index and harvest)
    Hooks(HooksArgs),
    /// Create and track codebase plans as markdown checklists in docs/plans/
    Plan(PlanArgs),
    /// Manage spec files: link human-authored docs to the code they govern
    Spec(SpecArgs),
    /// Agentic search loop: explore the codebase with iterative tool calls
    Explore(ExploreArgs),
    /// Manage and inspect cross-project links
    Links(LinksArgs),
    /// Manage historical code snapshots (index at a specific commit)
    Snapshot(SnapshotArgs),
    /// Show how a symbol evolved across indexed snapshots
    History(HistoryArgs),
    /// Low-level plumbing commands for agents and scripts (NDJSON output)
    Plumbing(PlumbingArgs),
}
