-- Spec files: human-authored markdown documents that govern code paths.
-- A spec is linked to one or more file/directory prefixes; when search
-- returns results from a linked path the governing specs are surfaced.

CREATE TABLE IF NOT EXISTS specs (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    path    TEXT    NOT NULL UNIQUE,   -- path as indexed (relative to project root)
    title   TEXT    NOT NULL DEFAULT '',
    is_auto INTEGER NOT NULL DEFAULT 0 -- 1 = auto-discovered by convention / frontmatter
);

CREATE TABLE IF NOT EXISTS spec_links (
    spec_id     INTEGER NOT NULL REFERENCES specs(id) ON DELETE CASCADE,
    linked_path TEXT    NOT NULL,      -- file path or directory prefix (e.g. "src/auth/")
    PRIMARY KEY (spec_id, linked_path)
);
