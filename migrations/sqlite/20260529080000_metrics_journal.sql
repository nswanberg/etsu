ALTER TABLE metrics ADD COLUMN journal_id TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_metrics_journal_id
ON metrics(journal_id)
WHERE journal_id IS NOT NULL;
