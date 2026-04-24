use anyhow::Result;

use super::Database;

/// A graph edge as returned by query methods.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GraphEdge {
    pub source_file: String,
    pub source_name: Option<String>,
    pub target_name: String,
    pub kind: String,
    pub line: usize,
}

fn row_to_edge(row: &rusqlite::Row<'_>) -> rusqlite::Result<GraphEdge> {
    Ok(GraphEdge {
        source_file: row.get(0)?,
        source_name: row.get(1)?,
        target_name: row.get(2)?,
        kind: row.get(3)?,
        line: row.get::<_, i64>(4)? as usize,
    })
}

impl Database {
    /// Insert a batch of edges for one file. Existing rows for that file are
    /// removed first (called during re-index).
    pub fn replace_edges(
        &self,
        file_path: &str,
        edges: &[crate::indexer::graph::Edge],
    ) -> Result<()> {
        self.conn.execute(
            "DELETE FROM graph_edges WHERE source_file = ?1",
            rusqlite::params![file_path],
        )?;
        for e in edges {
            self.conn.execute(
                "INSERT INTO graph_edges (source_file, source_name, target_name, kind, line)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    e.source_file,
                    e.source_name,
                    e.target_name,
                    e.kind.to_string(),
                    e.line as i64
                ],
            )?;
        }
        Ok(())
    }

    /// All edges where `name` appears as source_name OR target_name.
    pub fn edges_for_symbol(&self, name: &str) -> Result<Vec<GraphEdge>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT source_file, source_name, target_name, kind, line
             FROM graph_edges
             WHERE source_name = ?1 OR target_name = ?1
             ORDER BY kind, target_name",
        )?;
        let rows = stmt.query_map(rusqlite::params![name], row_to_edge)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// All edges originating from `file_path`.
    pub fn edges_for_file(&self, file_path: &str) -> Result<Vec<GraphEdge>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT source_file, source_name, target_name, kind, line
             FROM graph_edges
             WHERE source_file = ?1
             ORDER BY kind, target_name",
        )?;
        let rows = stmt.query_map(rusqlite::params![file_path], row_to_edge)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Return chunk IDs of symbols that are called-by or call the given chunk names.
    /// Used by `spelunk ask` to enrich context with graph neighbours.
    pub fn graph_neighbor_chunks(&self, names: &[&str]) -> Result<Vec<i64>> {
        if names.is_empty() {
            return Ok(vec![]);
        }
        let placeholders = names
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT DISTINCT c.id
             FROM chunks c
             WHERE c.name IN (
                 SELECT target_name FROM graph_edges
                 WHERE source_name IN ({placeholders}) AND kind = 'calls'
                 UNION
                 SELECT source_name FROM graph_edges
                 WHERE target_name IN ({placeholders}) AND kind = 'calls'
             )"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> =
            names.iter().map(|n| n as &dyn rusqlite::ToSql).collect();
        let rows = stmt.query_map(params.as_slice(), |r| r.get::<_, i64>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Return all (source_name, target_name) pairs from graph_edges where
    /// source_name is non-NULL. Used by PageRank computation after indexing.
    /// Excludes 'mentions' edges — those are for LinearRAG, not structural PageRank.
    pub fn graph_edges_all(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT source_name, target_name FROM graph_edges \
             WHERE source_name IS NOT NULL AND kind != 'mentions'",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Append mention edges for a file's chunks (without deleting — caller must have
    /// already called `replace_edges` which clears all edge kinds including 'mentions').
    pub fn append_mention_edges(
        &self,
        file_path: &str,
        edges: &[(Option<&str>, &str)],
    ) -> Result<()> {
        for (source_name, target_name) in edges {
            self.conn.execute(
                "INSERT INTO graph_edges (source_file, source_name, target_name, kind, line) \
                 VALUES (?1, ?2, ?3, 'mentions', 0)",
                rusqlite::params![file_path, source_name, target_name],
            )?;
        }
        Ok(())
    }

    /// For each chunk in `chunk_ids`, return the symbols it mentions.
    /// Joins via source_name + source_file — only works for named chunks.
    pub fn mention_edges_for_chunks(
        &self,
        chunk_ids: &[i64],
    ) -> Result<std::collections::HashMap<i64, Vec<String>>> {
        if chunk_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let placeholders = chunk_ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT c.id, ge.target_name
             FROM graph_edges ge
             JOIN chunks c ON c.name = ge.source_name
             JOIN files f ON f.id = c.file_id AND f.path = ge.source_file
             WHERE c.id IN ({placeholders})
               AND ge.kind IN ('mentions', 'calls')"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> = chunk_ids
            .iter()
            .map(|id| id as &dyn rusqlite::ToSql)
            .collect();
        let rows = stmt.query_map(params.as_slice(), |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut map: std::collections::HashMap<i64, Vec<String>> = std::collections::HashMap::new();
        for row in rows {
            let (chunk_id, symbol) = row?;
            map.entry(chunk_id).or_default().push(symbol);
        }
        Ok(map)
    }

    /// For each symbol in `symbols`, return the chunk IDs that mention it.
    pub fn chunks_mentioning_symbols(
        &self,
        symbols: &[&str],
    ) -> Result<std::collections::HashMap<String, Vec<i64>>> {
        if symbols.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let placeholders = symbols
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT ge.target_name, c.id
             FROM graph_edges ge
             JOIN chunks c ON c.name = ge.source_name
             JOIN files f ON f.id = c.file_id AND f.path = ge.source_file
             WHERE ge.target_name IN ({placeholders})
               AND ge.kind IN ('mentions', 'calls')"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> =
            symbols.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
        let rows = stmt.query_map(params.as_slice(), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?;
        let mut map: std::collections::HashMap<String, Vec<i64>> = std::collections::HashMap::new();
        for row in rows {
            let (symbol, chunk_id) = row?;
            map.entry(symbol).or_default().push(chunk_id);
        }
        Ok(map)
    }
}
