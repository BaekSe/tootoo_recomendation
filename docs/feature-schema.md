# Feature Schema

This project treats market data and derived signals as a reproducible, date-partitioned snapshot keyed by `as_of_date`.

## Table: stock_features_daily

- Primary key: `(as_of_date, ticker)`
- Columns
  - `as_of_date` (date): partition key for reproducible reads
  - `ticker` (text): stable identifier (e.g., `KRX:005930`)
  - `name` (text): display name
  - `trading_value` (double): optional liquidity proxy used for filtering/sorting
  - `features` (jsonb): compact numeric feature map (string -> number)

## features (jsonb)

Rules:
- Values must be numeric (JSON number) only.
- Keep the set small and stable; the worker passes 200-500 candidates with these features to the LLM.

Current keys (initial stub importer):
- `ret_1d` (float): 1-day return, unitless (e.g., 0.012 = +1.2%)
- `mom_5d` (float): 5-day momentum, unitless
- `vol_20d` (float): 20-day volatility proxy, unitless
- `value_score` (float): normalized [0, 1] rank-like score (higher is "cheaper" in stub)

These keys are placeholders; real ingestion should preserve the "numeric-only, compact" contract.
