-- Compound indexes to speed up LinearRAG mention-edge lookups.
-- The single-column indexes already exist; these cover (name, kind) filters
-- used by mention_edges_for_chunks and chunks_mentioning_symbols.
CREATE INDEX IF NOT EXISTS graph_edges_source_name_kind ON graph_edges(source_name, kind);
CREATE INDEX IF NOT EXISTS graph_edges_target_name_kind ON graph_edges(target_name, kind);
