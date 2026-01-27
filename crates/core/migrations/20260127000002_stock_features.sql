-- Minimal daily feature table for building the LLM candidate universe.
-- This is intentionally compact and as_of_date-partitioned to preserve reproducibility.

CREATE TABLE IF NOT EXISTS stock_features_daily (
  as_of_date date NOT NULL,
  ticker text NOT NULL,
  name text NOT NULL,
  trading_value double precision,
  features jsonb NOT NULL DEFAULT '{}'::jsonb,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (as_of_date, ticker)
);

CREATE INDEX IF NOT EXISTS stock_features_daily_as_of_date_idx
  ON stock_features_daily (as_of_date);

CREATE INDEX IF NOT EXISTS stock_features_daily_as_of_date_trading_value_idx
  ON stock_features_daily (as_of_date, trading_value DESC);
