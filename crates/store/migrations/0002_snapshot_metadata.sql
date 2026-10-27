-- Warning totals and the version the analyzer suggested at upload time.
ALTER TABLE versions ADD COLUMN warning_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE versions ADD COLUMN suggested_version TEXT;
CREATE INDEX IF NOT EXISTS versions_project_latest ON versions (project_id, id DESC);
