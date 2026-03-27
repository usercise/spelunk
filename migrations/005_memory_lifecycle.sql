-- Phase 1: memory lifecycle
-- Add status and superseded_by to notes table.
-- SQLite does not support NOT NULL without a default in ALTER TABLE,
-- so we use DEFAULT 'active' to backfill existing rows.

ALTER TABLE notes ADD COLUMN status TEXT NOT NULL DEFAULT 'active';
ALTER TABLE notes ADD COLUMN superseded_by INTEGER REFERENCES notes(id);
