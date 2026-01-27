# tootoo_recomendation

## Purpose

Personal-only web app that generates **daily short-term (<= 1 week) stock recommendations** for the Korea market.

- Recommendation logic (analysis/selection/explanation): delegated to an LLM API (currently Anthropic)
- My responsibility: build/refresh financial DB, run the daily recommendation job, store snapshots, and render them in a dashboard

## Recommendation Policy

- Schedule: **once per day, after market close (EOD)**
- Scheduler: **GitHub Actions (cron)** triggers a one-shot worker run (preferred for integrity)
- Output: **Top 20**
- For each stock: **3 short lines of rationale** (plus optional risk/caveat text)

## Data Scope

### Financial DB (managed here)

- Coverage: **all KRX listings**
- Minimum (recommended) fields
  - Daily OHLCV + trading value
  - Flows (foreign/institution/retail): daily + cumulative
  - If possible: fundamentals such as PER/PBR
- Requirement: must support **as-of (date-based) queries** so recommendations can be reproduced from snapshots

### AI Input (candidate universe)

- Universe: KRX-all -> internal prefilter -> **200~500 candidates** passed to the LLM
- Liquidity filter: optional internal knob (default can be “include all”, while still allowing tightening)

### AI Output (what must be stored)

Store a daily snapshot of the job output.

For each recommended stock:

- `rank` (1~20)
- `rationale` (3 short lines)
- `generated_at`
- `as_of_date`
- Optional: `confidence` / `risk_notes` / `disclaimer`

## Product Requirements

### Dashboard (PC-first)

- “Today’s Top 20” list
- Stock detail view
  - DB-based info (chart/flows/metrics)
  - AI rationale (3 lines)
- Historical snapshots
  - Browse by date
  - Compare snapshots across days

### Ops / Maintenance

- No login/auth
- No analytics (GA etc.)
- Monitoring / health
  - Sentry (error tracking)
  - UptimeRobot via `/healthz` endpoint (or equivalent)

## Non-Functional Requirements

- Reproducibility: every recommendation must be re-renderable using **stored snapshots** and **as-of DB reads**
- LLM responses must be machine-parseable (assume **JSON** contract)
- Because this is not public-facing:
  - traffic optimization/caching/Redis are low priority
  - access control is minimal (if needed: private URL or basic auth-level protection)

## Fixed Infra Choices (confirmed)

- Backend: Railway
- Backend language: Rust
- Frontend: Vercel
- DB: Supabase (Postgres)
- Monitoring: Sentry + UptimeRobot
- AI: Anthropic API (keep provider swappable; OpenAI is a future option)

## Open Questions (to be decided later)

- Frontend framework (e.g., Next.js) and UI component strategy
- Rust web framework/runtime choice (e.g., Axum/Actix-web) and job scheduling strategy
- Data ingestion sources for KRX prices/flows/fundamentals
- Exact schema and snapshotting strategy (append-only vs. versioned views)
- LLM provider abstraction (interface, JSON enforcement, and a clean switch path to OpenAI later)

## Current Implementation Status

Rust workspace scaffold exists with a minimal API and worker entrypoint.

- Workspace layout
  - `Cargo.toml` (workspace)
  - `crates/core` (`tootoo_core`): shared types/config + LLM trait skeleton
  - `crates/api` (`tootoo_api`): axum server (currently only `/healthz`)
  - `crates/worker` (`tootoo_worker`): one-shot CLI (placeholder)
- Commands
  - API: `cargo run -p tootoo_api`
  - Worker (EOD): `cargo run -p tootoo_worker --release`
  - Worker (backfill): `cargo run -p tootoo_worker --release -- --as-of-date YYYY-MM-DD`
  - Worker (dry-run): `cargo run -p tootoo_worker -- --dry-run`
  - Worker (seed features stub): `cargo run -p tootoo_worker -- --ingest-features --ingest-size 500`
  - Worker (ingest external): `cargo run -p tootoo_worker -- --ingest-external --as-of-date YYYY-MM-DD`
  - Check: `cargo check`
- Environment (WIP)
  - `ANTHROPIC_API_KEY` (LLM)
  - `DATABASE_URL` (Postgres connection string; Supabase)
  - `WORKER_DATABASE_URL` (optional; overrides DB connection for worker only)
  - `SENTRY_DSN` (optional)
  - Optional
    - `ANTHROPIC_MODEL` (example: `claude-3-5-sonnet-20241022`)
    - `ANTHROPIC_MAX_TOKENS` (default: `2048`)
    - `ANTHROPIC_BASE_URL` (default: `https://api.anthropic.com`)
    - `ANTHROPIC_TIMEOUT_SECS` (default: `60`)
    - Worker / Universe
      - `UNIVERSE_SIZE` (default: `200`, must be 200..=500)
      - `UNIVERSE_MIN_TRADING_VALUE` (optional)
      - `TOOTOO_USE_STUB_UNIVERSE` (set to any value to bypass DB and use deterministic stub candidates)
    - External data provider (ingest)
      - `DATA_PROVIDER_BASE_URL` (required for `--ingest-external`)
      - `DATA_PROVIDER_API_KEY` (optional; sent as `x-api-key`)
      - `DATA_PROVIDER_FEATURES_PATH` (default: `/v1/stock_features_daily`)
      - `DATA_PROVIDER_TIMEOUT_SECS` (default: `30`)
      - `DATA_PROVIDER_RETRIES` (default: `3`)
    - Market date
      - `KR_MARKET_HOLIDAYS` (optional CSV list: `YYYY-MM-DD,YYYY-MM-DD`)

## API

- `GET /healthz` -> `ok` (deterministic, does not call the LLM)
- `GET /snapshots/latest` -> latest successful snapshot (snapshot_id/provider + snapshot payload)
- `GET /snapshots/:as_of_date` -> successful snapshot for that date (YYYY-MM-DD)
- `GET /items/:as_of_date/:ticker` -> one item from that day's successful snapshot

## Runbook

- Snapshot semantics
  - Append-only: snapshots/items are inserted, never mutated.
  - Uniqueness: at most one `status='success'` snapshot per `as_of_date`.
  - Reproducibility: API reads are keyed by `as_of_date` and use stored snapshot records.
- Idempotency
  - Worker uses a Postgres advisory lock keyed by `as_of_date` to avoid concurrent runs.
  - DB also enforces a unique index for successful snapshots per `as_of_date`.
- Backfill
  - `cargo run -p tootoo_worker --release -- --as-of-date YYYY-MM-DD`
  - If a successful snapshot already exists for that date, the worker exits (no-op) and does not call the LLM.

## TODOs / Next Steps

If you resume work in another environment, this is the intended order.

1. Implement provider-agnostic LLM interface in core + Anthropic provider (strict JSON schema, 1x repair, diagnostics)
2. Implement worker EOD runner (as_of_date resolution, advisory lock/idempotency, transactional persist)
3. Implement backend API (healthz, latest snapshot, snapshot by date, stock detail endpoints)
4. Add GitHub Actions cron to run worker EOD job with secrets + retries
5. Add basic observability (tracing + Sentry hooks; healthz deterministic)
6. Add minimal frontend placeholder (optional) or document API contracts for Vercel UI

## Git Notes

This directory was initialized as a new git repository locally.

- Local commits exist, but there is no remote configured yet.
- To push from a new machine, add a remote and push:

```bash
git remote add origin <YOUR_GIT_REMOTE_URL>
git push -u origin main
```
