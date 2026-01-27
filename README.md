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
  - Worker: `cargo run -p tootoo_worker -- --as-of-date YYYY-MM-DD`
  - Check: `cargo check`
- Environment (WIP)
  - `ANTHROPIC_API_KEY` (LLM)
  - `DATABASE_URL` (Postgres connection string; Supabase)
  - `SENTRY_DSN` (optional)
