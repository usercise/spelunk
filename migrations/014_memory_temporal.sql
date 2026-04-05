-- Memory temporal fields: valid_at and invalid_at
-- Both are nullable TIMESTAMP columns (stored as INTEGER unix epoch, or NULL).
-- valid_at: when this entry became valid (defaults to created_at at app layer).
-- invalid_at: when this entry was superseded/invalidated (NULL = still valid).
-- No data migration needed: NULL is the correct default for existing rows.

ALTER TABLE notes ADD COLUMN valid_at INTEGER;
ALTER TABLE notes ADD COLUMN invalid_at INTEGER;

CREATE INDEX IF NOT EXISTS idx_memory_invalid_at ON notes(invalid_at);
