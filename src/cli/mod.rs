use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

pub mod cmd;

/// spelunk — local code intelligence
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Path to config file (default: ~/.config/spelunk/config.toml)
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
}

#[derive(Args, Debug)]
pub struct IndexArgs {
    /// Path to the codebase root to index
    pub path: PathBuf,

    /// Path to the SQLite database (overrides config)
    #[arg(short, long)]
    pub db: Option<PathBuf>,

    /// Max concurrent embedding requests (default: 32)
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

    /// Output format: text or json
    #[arg(long, default_value = "text")]
    pub format: String,
}

#[derive(Args, Debug)]
pub struct CheckArgs {
    /// Output format: text or json
    #[arg(long, default_value = "text")]
    pub format: String,

    /// Path to the SQLite database (overrides auto-detect)
    #[arg(short, long)]
    pub db: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct VerifyArgs {
    /// File path or symbol name to verify
    pub target: String,

    /// Number of nearest neighbours to show per chunk
    #[arg(long, default_value = "3")]
    pub neighbours: usize,

    /// Output format: text or json
    #[arg(long, default_value = "text")]
    pub format: String,

    /// Path to the SQLite database (overrides auto-detect)
    #[arg(short, long)]
    pub db: Option<PathBuf>,
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
    /// Store a decision, context, requirement, note, question, answer, or handoff
    Add(MemoryAddArgs),
    /// Semantic search over stored memory
    Search(MemorySearchArgs),
    /// List memory entries (newest first)
    List(MemoryListArgs),
    /// Show the full content of a memory entry
    Show(MemoryShowArgs),
    /// Auto-harvest memory entries from git commit messages using the LLM
    Harvest(MemoryHarvestArgs),
    /// Archive a memory entry (hidden from search and ask, but preserved)
    Archive(MemoryArchiveArgs),
    /// Archive an entry and mark it as superseded by a newer entry
    Supersede(MemorySupersededArgs),
    /// Push all local memory entries to the configured memory server
    Push(MemoryPushArgs),
}

#[derive(Args, Debug)]
pub struct MemoryAddArgs {
    /// Short title summarising the entry (inferred from URL if --from-url is used)
    #[arg(short, long)]
    pub title: Option<String>,

    /// Full body text (omit to open $EDITOR)
    #[arg(short, long)]
    pub body: Option<String>,

    /// Fetch content from a URL (GitHub issue, Linear ticket, or any web page)
    #[arg(long)]
    pub from_url: Option<String>,

    /// Kind: decision, context, requirement, note, question, answer, handoff
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

    /// Include archived entries
    #[arg(long)]
    pub archived: bool,
}

#[derive(Args, Debug)]
pub struct MemoryShowArgs {
    /// Entry ID (from list or search output)
    pub id: i64,

    /// Output format: text or json
    #[arg(long, default_value = "text")]
    pub format: String,
}

#[derive(Args, Debug)]
pub struct MemoryHarvestArgs {
    /// Git revision range to analyse (default: HEAD~10..HEAD)
    #[arg(long, default_value = "HEAD~10..HEAD")]
    pub git_range: String,
}

#[derive(Args, Debug)]
pub struct MemoryPushArgs {
    /// Local memory.db to push from (default: same as --db)
    #[arg(long)]
    pub source: Option<std::path::PathBuf>,
    /// Push archived entries too
    #[arg(long)]
    pub include_archived: bool,
}

#[derive(Args, Debug)]
pub struct MemoryArchiveArgs {
    /// ID of the entry to archive (from `spelunk memory list`)
    pub id: i64,
}

#[derive(Args, Debug)]
pub struct MemorySupersededArgs {
    /// ID of the entry to archive (the outdated one)
    pub old_id: i64,
    /// ID of the entry that replaces it (the new one)
    pub new_id: i64,
}

// ── Hooks ─────────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct HooksArgs {
    #[command(subcommand)]
    pub command: HooksCommand,
}

#[derive(Subcommand, Debug)]
pub enum HooksCommand {
    /// Install a post-commit hook that auto-indexes and harvests memory
    Install(HooksInstallArgs),
    /// Remove the spelunk post-commit hook
    Uninstall,
}

#[derive(Args, Debug)]
pub struct HooksInstallArgs {
    /// Print a GitHub Actions workflow step instead of writing a git hook
    #[arg(long)]
    pub ci: bool,
}

// ── Plan ──────────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct PlanArgs {
    #[command(subcommand)]
    pub command: PlanCommand,

    /// Path to the SQLite database (overrides auto-detect)
    #[arg(long, global = true)]
    pub db: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
pub enum PlanCommand {
    /// Create a new plan from a description (queries codebase + memory)
    Create(PlanCreateArgs),
    /// Show completion status of plans in docs/plans/
    Status(PlanStatusArgs),
}

#[derive(Args, Debug)]
pub struct PlanCreateArgs {
    /// Description of the task to plan
    pub description: String,

    /// Override the auto-generated filename slug
    #[arg(long)]
    pub name: Option<String>,
}

#[derive(Args, Debug)]
pub struct PlanStatusArgs {
    /// Show only this plan (by filename stem, e.g. add-rate-limiting)
    pub name: Option<String>,

    /// Output format: text or json
    #[arg(long, default_value = "text")]
    pub format: String,
}

// ── Spec ──────────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct SpecArgs {
    #[command(subcommand)]
    pub command: SpecCommand,

    /// Path to the SQLite database (overrides auto-detect)
    #[arg(long, global = true)]
    pub db: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
pub enum SpecCommand {
    /// Link a spec file to one or more code paths it governs
    Link(SpecLinkArgs),
    /// Remove a link between a spec and a code path
    Unlink(SpecUnlinkArgs),
    /// List all registered spec files and their links
    List(SpecListArgs),
    /// Show specs whose linked code has been re-indexed since the spec was last indexed
    Check(SpecCheckArgs),
}

#[derive(Args, Debug)]
pub struct SpecLinkArgs {
    /// Path to the spec file (markdown)
    pub spec: PathBuf,

    /// One or more file paths or directory prefixes this spec governs
    #[arg(required = true)]
    pub paths: Vec<String>,
}

#[derive(Args, Debug)]
pub struct SpecUnlinkArgs {
    /// Path to the spec file
    pub spec: PathBuf,

    /// Path prefix to remove (leave empty to remove all links for this spec)
    pub path: Option<String>,
}

#[derive(Args, Debug)]
pub struct SpecListArgs {
    /// Output format: text or json
    #[arg(long, default_value = "text")]
    pub format: String,
}

#[derive(Args, Debug)]
pub struct SpecCheckArgs {
    /// Output format: text or json
    #[arg(long, default_value = "text")]
    pub format: String,
}
