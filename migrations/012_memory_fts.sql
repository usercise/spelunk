-- FTS5 full-text index for memory notes.
-- Uses content= to avoid duplicating data; triggers keep it in sync.

CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
    title,
    body,
    tags,
    content=notes,
    content_rowid=id
);

-- Populate index from any existing rows.
INSERT OR IGNORE INTO memory_fts(rowid, title, body, tags)
SELECT id, title, body, COALESCE(tags, '') FROM notes;

-- Keep memory_fts in sync with notes table.

CREATE TRIGGER IF NOT EXISTS memory_fts_insert
AFTER INSERT ON notes BEGIN
    INSERT INTO memory_fts(rowid, title, body, tags)
    VALUES (new.id, new.title, new.body, COALESCE(new.tags, ''));
END;

CREATE TRIGGER IF NOT EXISTS memory_fts_delete
BEFORE DELETE ON notes BEGIN
    INSERT INTO memory_fts(memory_fts, rowid, title, body, tags)
    VALUES ('delete', old.id, old.title, old.body, COALESCE(old.tags, ''));
END;

CREATE TRIGGER IF NOT EXISTS memory_fts_update
AFTER UPDATE ON notes BEGIN
    INSERT INTO memory_fts(memory_fts, rowid, title, body, tags)
    VALUES ('delete', old.id, old.title, old.body, COALESCE(old.tags, ''));
    INSERT INTO memory_fts(rowid, title, body, tags)
    VALUES (new.id, new.title, new.body, COALESCE(new.tags, ''));
END;
