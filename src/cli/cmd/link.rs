use anyhow::{Context, Result};

use crate::{
    config::Config,
    registry::Registry,
};
use super::super::{LinkArgs, UnlinkArgs};

pub fn link(args: LinkArgs, _cfg: Config) -> Result<()> {
    let cwd = std::env::current_dir().context("getting current directory")?;
    let reg = Registry::open().context("opening registry")?;

    // Resolve current project
    let primary = reg.find_project_for_path(&cwd)?.with_context(|| {
        "No indexed project found for the current directory.\n\
             Run `spelunk index .` first."
            .to_string()
    })?;

    // Resolve target
    let target_path = if args.path.is_absolute() {
        args.path.clone()
    } else {
        cwd.join(&args.path)
    };
    let target_canonical = target_path
        .canonicalize()
        .unwrap_or_else(|_| target_path.clone());

    if target_canonical == primary.root_path {
        anyhow::bail!("A project cannot depend on itself.");
    }

    let dep = reg
        .find_project_for_path(&target_canonical)?
        .with_context(|| {
            format!(
                "No index found for '{}'.\n\
             Run `spelunk index {}` first.",
                target_canonical.display(),
                target_canonical.display()
            )
        })?;

    reg.add_dep(primary.id, dep.id)?;

    println!(
        "Linked: {} → {}",
        primary.root_path.display(),
        dep.root_path.display()
    );
    println!(
        "Searches from '{}' will now include results from '{}'.",
        primary.root_path.display(),
        dep.root_path.display()
    );
    Ok(())
}

pub fn unlink(args: UnlinkArgs, _cfg: Config) -> Result<()> {
    let cwd = std::env::current_dir().context("getting current directory")?;
    let reg = Registry::open().context("opening registry")?;

    let primary = reg
        .find_project_for_path(&cwd)?
        .with_context(|| "No indexed project found for the current directory.")?;

    let target_path = if args.path.is_absolute() {
        args.path.clone()
    } else {
        cwd.join(&args.path)
    };
    let target_canonical = target_path
        .canonicalize()
        .unwrap_or_else(|_| target_path.clone());

    let dep = reg.find_by_root(&target_canonical)?.with_context(|| {
        format!(
            "No registered project found at '{}'.",
            target_canonical.display()
        )
    })?;

    reg.remove_dep(primary.id, dep.id)?;

    println!(
        "Unlinked: {} ↛ {}",
        primary.root_path.display(),
        dep.root_path.display()
    );
    Ok(())
}

pub fn autoclean(_cfg: Config) -> Result<()> {
    let reg = Registry::open().context("opening registry")?;
    let removed = reg.autoclean()?;
    if removed.is_empty() {
        println!(
            "All {} registered project(s) have valid paths — nothing to clean.",
            reg.all_projects()?.len()
        );
    } else {
        println!("Removed {} stale project(s):", removed.len());
        for path in &removed {
            println!("  - {path}");
        }
    }
    Ok(())
}
