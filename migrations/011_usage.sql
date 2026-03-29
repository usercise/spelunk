CREATE TABLE IF NOT EXISTS usage (
    command   TEXT    NOT NULL,
    called_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_usage_called_at ON usage(called_at);
