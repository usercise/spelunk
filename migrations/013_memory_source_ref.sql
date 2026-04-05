-- Add commit provenance to memory entries.
-- source_ref stores the full 40-character git SHA for harvested entries;
-- NULL for entries created manually.
ALTER TABLE notes ADD COLUMN source_ref TEXT;
