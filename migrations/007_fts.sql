CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
    name,
    content,
    node_type,
    content=chunks,
    content_rowid=id
);

-- Keep FTS in sync with chunks table
CREATE TRIGGER IF NOT EXISTS chunks_fts_insert
AFTER INSERT ON chunks BEGIN
    INSERT INTO chunks_fts(rowid, name, content, node_type)
    VALUES (new.id, new.name, new.content, new.node_type);
END;

CREATE TRIGGER IF NOT EXISTS chunks_fts_delete
BEFORE DELETE ON chunks BEGIN
    INSERT INTO chunks_fts(chunks_fts, rowid, name, content, node_type)
    VALUES ('delete', old.id, old.name, old.content, old.node_type);
END;

CREATE TRIGGER IF NOT EXISTS chunks_fts_update
AFTER UPDATE ON chunks BEGIN
    INSERT INTO chunks_fts(chunks_fts, rowid, name, content, node_type)
    VALUES ('delete', old.id, old.name, old.content, old.node_type);
    INSERT INTO chunks_fts(rowid, name, content, node_type)
    VALUES (new.id, new.name, new.content, new.node_type);
END;
