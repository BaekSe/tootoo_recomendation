-- Append-only snapshot storage for daily recommendations.

CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE IF NOT EXISTS recommendation_snapshots (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  as_of_date date NOT NULL,
  generated_at timestamptz NOT NULL,
  provider text NOT NULL,
  status text NOT NULL,
  error text,
  raw_llm_response jsonb,
  created_at timestamptz NOT NULL DEFAULT now()
);

-- Ensure at most one SUCCESS snapshot per as-of date.
CREATE UNIQUE INDEX IF NOT EXISTS recommendation_snapshots_success_unique
  ON recommendation_snapshots (as_of_date)
  WHERE status = 'success';

CREATE INDEX IF NOT EXISTS recommendation_snapshots_as_of_date_idx
  ON recommendation_snapshots (as_of_date);

CREATE TABLE IF NOT EXISTS recommendation_items (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  snapshot_id uuid NOT NULL REFERENCES recommendation_snapshots (id) ON DELETE RESTRICT,
  rank int NOT NULL,
  ticker text NOT NULL,
  name text NOT NULL,
  rationale text[] NOT NULL,
  risk_notes text,
  confidence double precision,
  created_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT recommendation_items_rank_range CHECK (rank BETWEEN 1 AND 20),
  CONSTRAINT recommendation_items_rationale_len CHECK (array_length(rationale, 1) = 3),
  CONSTRAINT recommendation_items_confidence_range CHECK (
    confidence IS NULL OR (confidence >= 0.0 AND confidence <= 1.0)
  )
);

CREATE UNIQUE INDEX IF NOT EXISTS recommendation_items_snapshot_rank_unique
  ON recommendation_items (snapshot_id, rank);

CREATE UNIQUE INDEX IF NOT EXISTS recommendation_items_snapshot_ticker_unique
  ON recommendation_items (snapshot_id, ticker);

CREATE INDEX IF NOT EXISTS recommendation_items_ticker_idx
  ON recommendation_items (ticker);
