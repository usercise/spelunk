//! Clap argument structs for all spelunk subcommands.
//! Re-exported from `cli::mod` so callers can use `crate::cli::XxxArgs`.
use clap::{Args, Subcommand};
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct InitArgs {
    /// Also install the post-commit git hook
    #[arg(long)]
    pub hook: bool,

    /// Skip the initial index run
    #[arg(long)]
    pub no_index: bool,
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

    /// Backfill token_count for all existing chunks and exit (useful for upgrading old indexes)
    #[arg(long)]
    pub recount: bool,

    /// Skip LLM summary generation even when llm_model is configured
    #[arg(long)]
    pub no_summaries: bool,

    /// Number of chunks to send to the LLM per summary request (default: 10)
    #[arg(long, default_value = "10")]
    pub summary_batch_size: usize,

    /// Internal: run only phases 3-5 (graph rank, spec discovery, summaries).
    /// Used by the background process spawned after a large foreground index.
    #[arg(long = "_background-phases", hide = true, default_value_t = false)]
    pub background_phases: bool,
}

#[derive(Args, Debug)]
pub struct SearchArgs {
    /// Natural language search query
    pub query: String,

    /// Number of results to return (max 100)
    #[arg(short, long, default_value = "10", conflicts_with = "budget")]
    pub limit: usize,

    /// Return best chunks fitting within this token budget (mutually exclusive with --limit)
    #[arg(long, conflicts_with = "limit")]
    pub budget: Option<usize>,

    /// Output format: text or json
    #[arg(long, default_value = "text")]
    pub format: String,

    /// Enrich results with 1-hop call-graph neighbours (callers + callees)
    #[arg(short, long)]
    pub graph: bool,

    /// Maximum number of graph-expanded results to add (when --graph is set)
    #[arg(long, default_value = "10")]
    pub graph_limit: usize,

    /// Search mode: hybrid (default), semantic, text
    #[arg(long, default_value = "hybrid")]
    pub mode: String,

    /// Path to the SQLite database (overrides config)
    #[arg(short, long)]
    pub db: Option<PathBuf>,

    /// Skip the lightweight staleness probe (suppress stale-index warning)
    #[arg(long)]
    pub no_stale_check: bool,

    /// Search only the primary project index, skipping all linked project DBs
    #[arg(long)]
    pub local_only: bool,

    /// Search against this snapshot instead of the live index (full or short commit SHA)
    #[arg(long, value_name = "SHA")]
    pub as_of: Option<String>,
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

    /// Skip the lightweight staleness probe (suppress stale-index warning)
    #[arg(long)]
    pub no_stale_check: bool,
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

    /// Skip the lightweight staleness probe (suppress stale-index warning)
    #[arg(long)]
    pub no_stale_check: bool,
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

    /// List the stale file paths (one per line) in addition to the summary
    #[arg(long)]
    pub files: bool,

    /// Machine-readable output: `stale=N total=M last_indexed=T`
    #[arg(long)]
    pub porcelain: bool,
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
pub struct ExploreArgs {
    /// The question to answer about the codebase
    pub question: String,

    /// Path to the SQLite database (overrides config)
    #[arg(short, long)]
    pub db: Option<PathBuf>,

    /// Maximum number of tool-call steps before forcing a final answer
    #[arg(long, default_value_t = 10)]
    pub max_steps: usize,

    /// Print each tool call and result to stderr as they happen
    #[arg(long)]
    pub verbose: bool,

    /// Output result as JSON (answer + sources + step log)
    #[arg(long)]
    pub json: bool,
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
    /// Show how the team's understanding of a topic evolved over time
    Timeline(MemoryTimelineArgs),
    /// Show the relationship graph for a memory entry
    Graph(MemoryGraphArgs),
}

#[derive(Args, Debug)]
pub struct MemoryGraphArgs {
    /// Entry ID to show the relationship graph for
    pub id: i64,

    /// Output format: text or json
    #[arg(long, default_value = "text")]
    pub format: String,
}

#[derive(Args, Debug)]
pub struct MemoryTimelineArgs {
    /// Topic to trace through time
    pub query: String,

    /// Number of entries to retrieve before timeline construction
    #[arg(short, long, default_value = "20")]
    pub limit: usize,

    /// Output format: text or json
    #[arg(long, default_value = "text")]
    pub format: String,
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

    /// When this entry became valid (ISO 8601, e.g. 2026-03-15 or 2026-03-15T10:00:00).
    /// Defaults to now (created_at) when omitted.
    #[arg(long, value_name = "DATE")]
    pub valid_at: Option<String>,

    /// ID of an existing entry that this new entry supersedes.
    /// The old entry's invalid_at is set to now atomically in the same transaction.
    #[arg(long, value_name = "ID")]
    pub supersedes: Option<i64>,

    /// ID of an existing entry this entry relates to (creates a relates_to edge).
    #[arg(long, value_name = "ID")]
    pub relates_to: Option<i64>,
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

    /// Search mode: hybrid (default), semantic, text
    #[arg(long, default_value = "hybrid")]
    pub mode: String,

    /// Return only entries valid at this point in time (ISO 8601, e.g. 2026-03-15 or 2026-03-15T10:00:00)
    #[arg(long, value_name = "DATE")]
    pub as_of: Option<String>,

    /// Expand results by 1 hop along relates_to edges
    #[arg(long)]
    pub expand_graph: bool,
}

#[derive(Args, Debug)]
pub struct MemoryListArgs {
    /// Filter by kind: decision, context, requirement, note
    #[arg(short, long)]
    pub kind: Option<String>,

    /// Filter by commit SHA (exact or prefix match against source_ref)
    #[arg(long)]
    pub source_ref: Option<String>,

    /// Number of entries to show
    #[arg(short, long, default_value = "20")]
    pub limit: usize,

    /// Output format: text or json
    #[arg(long, default_value = "text")]
    pub format: String,

    /// Include archived entries
    #[arg(long)]
    pub archived: bool,

    /// Return only entries valid at this point in time (ISO 8601, e.g. 2026-03-15 or 2026-03-15T10:00:00)
    #[arg(long, value_name = "DATE")]
    pub as_of: Option<String>,
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
    /// Git revision range to analyse, e.g. `HEAD~10..HEAD` or `v0.1.0..HEAD`.
    /// Mutually exclusive with --branch.
    #[arg(long, default_value = "HEAD~10..HEAD", conflicts_with = "branch")]
    pub git_range: String,

    /// Harvest the entire commit history of a branch, e.g. `main` or `master`.
    /// Mutually exclusive with --git-range.
    #[arg(long, conflicts_with = "git_range")]
    pub branch: Option<String>,

    /// Number of commits to send to the LLM in each request.
    /// Smaller values are more stable; larger values risk hitting context-window limits.
    #[arg(long, default_value_t = 3)]
    pub batch_size: usize,
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

// ── Links ─────────────────────────────────────────────────────────────────────

/// Manage and inspect cross-project links.
/// Use `spelunk link <path>` / `spelunk unlink <path>` to add or remove links.
#[derive(Args, Debug)]
pub struct LinksArgs {
    #[command(subcommand)]
    pub command: LinksCommand,
}

#[derive(Subcommand, Debug)]
pub enum LinksCommand {
    /// List all linked projects with their status
    List(LinksListArgs),
    /// Check all linked project indexes are fresh (exit 1 if any are stale or missing)
    Check,
}

#[derive(Args, Debug)]
pub struct LinksListArgs {
    /// Output format: text or json
    #[arg(long, default_value = "text")]
    pub format: String,
}

// ── Snapshot ───────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct SnapshotArgs {
    #[command(subcommand)]
    pub command: SnapshotCommand,

    /// Path to the SQLite database (overrides auto-detect)
    #[arg(long, global = true)]
    pub db: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
pub enum SnapshotCommand {
    /// Index the codebase at a specific commit and store as a named snapshot
    Create(SnapshotCreateArgs),
    /// List all stored snapshots
    List(SnapshotListArgs),
    /// Delete a snapshot and its data
    Delete(SnapshotDeleteArgs),
}

#[derive(Args, Debug)]
pub struct SnapshotCreateArgs {
    /// Git commit SHA or ref to snapshot (defaults to HEAD)
    #[arg(default_value = "HEAD")]
    pub commit: String,

    /// Max concurrent embedding requests
    #[arg(long, default_value = "32")]
    pub batch_size: usize,
}

#[derive(Args, Debug)]
pub struct SnapshotListArgs {
    /// Output format: text or json
    #[arg(long, default_value = "text")]
    pub format: String,
}

#[derive(Args, Debug)]
pub struct SnapshotDeleteArgs {
    /// Commit SHA of the snapshot to delete (full or short)
    pub sha: String,
}

// ── Plumbing ───────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct PlumbingArgs {
    #[command(subcommand)]
    pub command: PlumbingCommand,

    /// Path to the SQLite database (overrides auto-detect)
    #[arg(short, long, global = true)]
    pub db: Option<std::path::PathBuf>,
}

#[derive(Subcommand, Debug)]
pub enum PlumbingCommand {
    /// Emit indexed chunks for a file as NDJSON
    CatChunks(PlumbingCatChunksArgs),
    /// List all indexed files as NDJSON
    LsFiles(PlumbingLsFilesArgs),
    /// Parse a file and emit chunks as NDJSON (without storing)
    ParseFile(PlumbingParseFileArgs),
    /// Compute blake3 hash of a file and check index currency
    HashFile(PlumbingHashFileArgs),
    /// KNN vector search returning NDJSON results
    Knn(PlumbingKnnArgs),
    /// Read lines from stdin and emit embedding vectors as NDJSON
    Embed(PlumbingEmbedArgs),
    /// Emit code graph edges as NDJSON
    GraphEdges(PlumbingGraphEdgesArgs),
    /// Emit memory entries as NDJSON
    ReadMemory(PlumbingReadMemoryArgs),
}

#[derive(Args, Debug)]
pub struct PlumbingCatChunksArgs {
    /// Path of the file whose chunks to emit (relative to project root)
    pub file: String,
}

#[derive(Args, Debug)]
pub struct PlumbingLsFilesArgs {
    /// Only list files whose path starts with this prefix
    #[arg(long)]
    pub prefix: Option<String>,

    /// Only emit files where on-disk hash differs from stored hash
    #[arg(long)]
    pub stale: bool,
}

#[derive(Args, Debug)]
pub struct PlumbingParseFileArgs {
    /// Path to the file to parse
    pub file: std::path::PathBuf,
}

#[derive(Args, Debug)]
pub struct PlumbingHashFileArgs {
    /// Path to the file to hash
    pub file: std::path::PathBuf,
}

#[derive(Args, Debug)]
pub struct PlumbingKnnArgs {
    /// Maximum number of results (default: 10)
    #[arg(long, default_value = "10")]
    pub limit: usize,

    /// Drop results below this cosine similarity score
    #[arg(long, default_value = "0.0")]
    pub min_score: f32,

    /// Restrict results to chunks from files of this language
    #[arg(long)]
    pub lang: Option<String>,
}

#[derive(Args, Debug)]
pub struct PlumbingEmbedArgs {
    /// Prepend query retrieval prefix instead of document prefix
    #[arg(long)]
    pub query: bool,
}

#[derive(Args, Debug)]
pub struct PlumbingGraphEdgesArgs {
    /// Filter edges to those involving this file (path relative to project root)
    #[arg(long)]
    pub file: Option<String>,

    /// Filter edges to those involving this symbol name
    #[arg(long)]
    pub symbol: Option<String>,
}

#[derive(Args, Debug)]
pub struct PlumbingReadMemoryArgs {
    /// Filter by memory kind (decision, question, note, etc.)
    #[arg(long)]
    pub kind: Option<String>,

    /// Fetch a single entry by id
    #[arg(long)]
    pub id: Option<i64>,

    /// Maximum number of entries (default: 50)
    #[arg(long, default_value = "50")]
    pub limit: usize,
}

// ── History ────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct HistoryArgs {
    /// Symbol name to trace (function, struct, class, etc.)
    pub symbol: String,

    /// Output format: text or json
    #[arg(long, default_value = "text")]
    pub format: String,

    /// Path to the SQLite database (overrides auto-detect)
    #[arg(short, long)]
    pub db: Option<PathBuf>,
}
