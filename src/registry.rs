//! Global project registry.
//!
//! Stores all known project roots and their dependency relationships in a
//! single SQLite database at `~/.config/spelunk/registry.db`.
//!
//! The registry is separate from per-project index DBs.  It is used to:
//!   - Auto-detect which project the user is working in from their CWD
//!   - Track cross-project dependencies for multi-repo search
//!   - Power `spelunk status --all` and `spelunk autoclean`

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Project {
    pub id: i64,
    pub root_path: PathBuf,
    pub db_path: PathBuf,
    #[allow(dead_code)]
    pub registered_at: i64,
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

pub struct Registry {
    conn: Connection,
}

impl Registry {
    /// Open (or create) the global registry database.
    pub fn open() -> Result<Self> {
        let path = registry_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating registry directory {}", parent.display()))?;
        }
        let conn = Connection::open(&path)
            .with_context(|| format!("opening registry at {}", path.display()))?;
        let reg = Self { conn };
        reg.init()?;
        Ok(reg)
    }

    fn init(&self) -> Result<()> {
        self.conn.execute_batch("
            PRAGMA journal_mode=WAL;
            PRAGMA foreign_keys=ON;

            CREATE TABLE IF NOT EXISTS projects (
                id            INTEGER PRIMARY KEY AUTOINCREMENT,
                root_path     TEXT    NOT NULL UNIQUE,
                db_path       TEXT    NOT NULL,
                registered_at INTEGER NOT NULL DEFAULT (unixepoch())
            );

            CREATE TABLE IF NOT EXISTS project_deps (
                project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                dep_id     INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                PRIMARY KEY (project_id, dep_id)
            );
        ").context("initialising registry schema")?;
        Ok(())
    }

    // ── Registration ──────────────────────────────────────────────────────────

    /// Register (or update) a project.  Returns the project's id.
    pub fn register(&self, root: &Path, db: &Path) -> Result<i64> {
        let root_str = root.to_string_lossy();
        let db_str   = db.to_string_lossy();
        self.conn.execute(
            "INSERT INTO projects (root_path, db_path)
             VALUES (?1, ?2)
             ON CONFLICT(root_path) DO UPDATE SET db_path = excluded.db_path",
            params![root_str, db_str],
        ).context("registering project")?;
        let id: i64 = self.conn.query_row(
            "SELECT id FROM projects WHERE root_path = ?1",
            params![root_str],
            |row| row.get(0),
        ).context("fetching project id after register")?;
        Ok(id)
    }

    // ── Lookup ────────────────────────────────────────────────────────────────

    /// Find the closest ancestor of `start` that is a registered project root.
    /// If none found in the registry, falls back to filesystem walk looking for
    /// `.spelunk/index.db` and auto-registers what it finds.
    pub fn find_project_for_path(&self, start: &Path) -> Result<Option<Project>> {
        // 1. Registry walk-up (most specific first)
        let mut dir = start.to_path_buf();
        loop {
            let dir_str = dir.to_string_lossy().to_string();
            let maybe: Option<(i64, String, String, i64)> = self.conn.query_row(
                "SELECT id, root_path, db_path, registered_at
                 FROM projects WHERE root_path = ?1",
                params![dir_str],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            ).optional().context("querying registry")?;

            if let Some((id, root_path, db_path, registered_at)) = maybe {
                return Ok(Some(Project {
                    id,
                    root_path: PathBuf::from(root_path),
                    db_path: PathBuf::from(db_path),
                    registered_at,
                }));
            }
            if !dir.pop() { break; }
        }

        // 2. Filesystem fallback — look for .spelunk/index.db and auto-register.
        let mut dir = start.to_path_buf();
        loop {
            let candidate = dir.join(".spelunk").join("index.db");
            if candidate.exists() {
                let id = self.register(&dir, &candidate)?;
                return Ok(Some(Project {
                    id,
                    root_path: dir.clone(),
                    db_path: candidate,
                    registered_at: 0,
                }));
            }
            if !dir.pop() { break; }
        }

        Ok(None)
    }

    /// Find a project by its exact root path.
    pub fn find_by_root(&self, root: &Path) -> Result<Option<Project>> {
        let root_str = root.to_string_lossy().to_string();
        self.conn.query_row(
            "SELECT id, root_path, db_path, registered_at
             FROM projects WHERE root_path = ?1",
            params![root_str],
            |row| {
                Ok(Project {
                    id: row.get(0)?,
                    root_path: PathBuf::from(row.get::<_, String>(1)?),
                    db_path:   PathBuf::from(row.get::<_, String>(2)?),
                    registered_at: row.get(3)?,
                })
            },
        ).optional().context("querying registry by root")
    }

    // ── Dependencies ──────────────────────────────────────────────────────────

    /// Return all dep DB paths for a project (direct deps only).
    pub fn get_deps(&self, project_id: i64) -> Result<Vec<Project>> {
        let mut stmt = self.conn.prepare(
            "SELECT p.id, p.root_path, p.db_path, p.registered_at
             FROM projects p
             JOIN project_deps d ON d.dep_id = p.id
             WHERE d.project_id = ?1",
        ).context("preparing dep query")?;

        let rows = stmt.query_map(params![project_id], |row| {
            Ok(Project {
                id: row.get(0)?,
                root_path: PathBuf::from(row.get::<_, String>(1)?),
                db_path:   PathBuf::from(row.get::<_, String>(2)?),
                registered_at: row.get(3)?,
            })
        }).context("querying deps")?;

        rows.collect::<rusqlite::Result<Vec<_>>>().context("reading dep rows")
    }

    /// Add a dependency: `from_id` depends on `dep_id`.
    pub fn add_dep(&self, from_id: i64, dep_id: i64) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO project_deps (project_id, dep_id) VALUES (?1, ?2)",
            params![from_id, dep_id],
        ).context("adding dependency")?;
        Ok(())
    }

    /// Remove a dependency.
    pub fn remove_dep(&self, from_id: i64, dep_id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM project_deps WHERE project_id = ?1 AND dep_id = ?2",
            params![from_id, dep_id],
        ).context("removing dependency")?;
        Ok(())
    }

    // ── Listing ───────────────────────────────────────────────────────────────

    /// Return all registered projects, ordered by root_path.
    pub fn all_projects(&self) -> Result<Vec<Project>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, root_path, db_path, registered_at
             FROM projects ORDER BY root_path",
        ).context("preparing all-projects query")?;

        let rows = stmt.query_map([], |row| {
            Ok(Project {
                id: row.get(0)?,
                root_path: PathBuf::from(row.get::<_, String>(1)?),
                db_path:   PathBuf::from(row.get::<_, String>(2)?),
                registered_at: row.get(3)?,
            })
        }).context("querying all projects")?;

        rows.collect::<rusqlite::Result<Vec<_>>>().context("reading project rows")
    }

    /// Return projects that list `project_id` as a dependency (reverse deps).
    #[allow(dead_code)]
    pub fn projects_depending_on(&self, project_id: i64) -> Result<Vec<Project>> {
        let mut stmt = self.conn.prepare(
            "SELECT p.id, p.root_path, p.db_path, p.registered_at
             FROM projects p
             JOIN project_deps d ON d.project_id = p.id
             WHERE d.dep_id = ?1",
        ).context("preparing reverse-dep query")?;

        let rows = stmt.query_map(params![project_id], |row| {
            Ok(Project {
                id: row.get(0)?,
                root_path: PathBuf::from(row.get::<_, String>(1)?),
                db_path:   PathBuf::from(row.get::<_, String>(2)?),
                registered_at: row.get(3)?,
            })
        }).context("querying reverse deps")?;

        rows.collect::<rusqlite::Result<Vec<_>>>().context("reading reverse-dep rows")
    }

    // ── Autoclean ─────────────────────────────────────────────────────────────

    /// Remove all registry entries whose root path no longer exists on disk.
    /// Returns the list of removed root paths.
    pub fn autoclean(&self) -> Result<Vec<String>> {
        let projects = self.all_projects()?;
        let mut removed = Vec::new();
        for p in projects {
            if !p.root_path.exists() {
                self.conn.execute(
                    "DELETE FROM projects WHERE id = ?1",
                    params![p.id],
                ).context("deleting stale project from registry")?;
                removed.push(p.root_path.to_string_lossy().to_string());
            }
        }
        Ok(removed)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn registry_path() -> Result<PathBuf> {
    let base = dirs::config_dir()
        .context("could not determine user config directory")?
        .join("spelunk");
    Ok(base.join("registry.db"))
}

// Allow `.optional()` on query_row results without boilerplate.
trait OptionalExt<T> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error>;
}

impl<T> OptionalExt<T> for rusqlite::Result<T> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
