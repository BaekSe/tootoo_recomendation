CREATE TABLE IF NOT EXISTS stock_features_ingest_runs (
  id uuid PRIMARY KEY,
  as_of_date date NOT NULL,
  generated_at timestamptz NOT NULL,
  provider text NOT NULL,
  status text NOT NULL,
  error text,
  raw_response jsonb
);

CREATE INDEX IF NOT EXISTS stock_features_ingest_runs_as_of_date_idx
  ON stock_features_ingest_runs (as_of_date, generated_at DESC);
