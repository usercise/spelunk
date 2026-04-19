use anyhow::Result;
use clap::Args;
use std::path::PathBuf;

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

use super::helpers::open_project_db;
use super::search::maybe_warn_stale;
use super::ui::spinner;
use crate::{
    config::Config,
    search::explore::{ExploreResult, Explorer},
};

pub async fn explore(args: ExploreArgs, cfg: Config) -> Result<()> {
    if cfg.llm_model.is_none() {
        anyhow::bail!(
            "spelunk explore requires a chat model. \
             Set `llm_model` in ~/.config/spelunk/config.toml."
        );
    }

    let (db_path, _db) = open_project_db(args.db.as_deref(), &cfg.db_path)?;
    maybe_warn_stale(&db_path);
    crate::storage::record_usage_at(&db_path, "explore");

    let sp = spinner("Loading models…");
    let embedder = crate::backends::ActiveEmbedder::load(&cfg).await?;
    let llm = crate::backends::ActiveLlm::load(&cfg).await?;
    sp.finish_and_clear();

    let verbose = args.verbose || crate::utils::is_agent_mode();
    let use_json = args.json || crate::utils::is_agent_mode();

    if !use_json {
        eprintln!("Exploring: {}\n", args.question);
    }

    let explorer = Explorer::new(db_path.clone(), &embedder, &llm, args.max_steps, verbose);
    let result = explorer.explore(&args.question).await?;

    if use_json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        print_result(&result);
    }

    Ok(())
}

fn print_result(result: &ExploreResult) {
    println!("{}", result.answer);
    if !result.sources.is_empty() {
        println!("\nSources:");
        for src in &result.sources {
            println!("  {src}");
        }
    }
    if !result.steps.is_empty() {
        let tools: Vec<&str> = result.steps.iter().map(|s| s.tool.as_str()).collect();
        println!(
            "\n{} tool call(s): {}",
            result.steps.len(),
            tools.join(", ")
        );
    }
}
