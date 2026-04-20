use anyhow::Result;
use clap::Args;
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct GraphArgs {
    /// Symbol name or file path to look up in the graph
    pub symbol: String,

    /// Filter to a specific edge kind: imports, calls, extends, implements
    #[arg(long)]
    pub kind: Option<String>,

    /// Output format: text, json, or ndjson
    #[arg(long, default_value = "text")]
    pub format: String,

    /// Path to the SQLite database (overrides config)
    #[arg(short, long)]
    pub db: Option<PathBuf>,

    /// Skip the lightweight staleness probe (suppress stale-index warning)
    #[arg(long)]
    pub no_stale_check: bool,
}

use super::helpers::open_project_db;
use super::search::maybe_warn_stale;
use crate::config::Config;

pub fn graph(args: GraphArgs, cfg: Config) -> Result<()> {
    let (db_path, db) = open_project_db(args.db.as_deref(), &cfg.db_path)?;

    if !args.no_stale_check {
        maybe_warn_stale(&db_path);
    }
    let symbol = &args.symbol;

    // Decide whether the query looks like a file path or a symbol name.
    let mut edges = if symbol.contains('/')
        || symbol.contains('\\')
        || symbol.ends_with(".rs")
        || symbol.ends_with(".py")
        || symbol.ends_with(".go")
        || symbol.ends_with(".java")
        || symbol.ends_with(".ts")
        || symbol.ends_with(".js")
    {
        db.edges_for_file(symbol)?
    } else {
        db.edges_for_symbol(symbol)?
    };

    // Optional kind filter
    if let Some(kind) = &args.kind {
        edges.retain(|e| e.kind == *kind);
    }

    if edges.is_empty() {
        println!("No graph edges found for '{symbol}'.");
        return Ok(());
    }

    match crate::utils::effective_format(&args.format) {
        "json" => println!("{}", serde_json::to_string_pretty(&edges)?),
        "ndjson" => {
            for edge in &edges {
                println!("{}", serde_json::to_string(edge)?);
            }
        }
        _ => print_edges(&edges, symbol),
    }

    Ok(())
}

fn print_edges(edges: &[crate::storage::GraphEdge], query: &str) {
    // Group into outgoing (source) and incoming (target) edges.
    let outgoing: Vec<_> = edges
        .iter()
        .filter(|e| e.source_name.as_deref() == Some(query) || e.source_file == query)
        .collect();
    let incoming: Vec<_> = edges.iter().filter(|e| e.target_name == query).collect();
    let other: Vec<_> = edges
        .iter()
        .filter(|e| {
            e.source_name.as_deref() != Some(query)
                && e.source_file != query
                && e.target_name != query
        })
        .collect();

    if !outgoing.is_empty() {
        println!("\x1b[1mOutgoing from '{query}':\x1b[0m");
        for e in &outgoing {
            let loc = e.source_name.as_deref().unwrap_or(&e.source_file);
            println!(
                "  \x1b[33m{}\x1b[0m  {}  \x1b[2m({}:{})\x1b[0m",
                e.kind, e.target_name, loc, e.line
            );
        }
        println!();
    }
    if !incoming.is_empty() {
        println!("\x1b[1mIncoming to '{query}':\x1b[0m");
        for e in &incoming {
            let loc = e.source_name.as_deref().unwrap_or(&e.source_file);
            println!(
                "  \x1b[36m{}\x1b[0m  {}  \x1b[2m({}:{})\x1b[0m",
                e.kind, e.source_file, loc, e.line
            );
        }
        println!();
    }
    if !other.is_empty() {
        println!("\x1b[1mRelated edges:\x1b[0m");
        for e in &other {
            let loc = e.source_name.as_deref().unwrap_or(&e.source_file);
            println!(
                "  {} -- \x1b[33m{}\x1b[0m --> {}  \x1b[2m({}:{})\x1b[0m",
                loc, e.kind, e.target_name, e.source_file, e.line
            );
        }
    }
}
