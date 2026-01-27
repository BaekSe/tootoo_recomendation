# Agent Guide

This repo is for a **personal stock recommendation dashboard**. Agent changes must preserve reproducibility and snapshot semantics.

## Product Invariants

- Personal-only: no multi-user, no auth, no analytics
- Daily EOD job produces exactly one recommendation snapshot per day
- Every UI view must be reproducible from:
  - stored recommendation snapshots (immutable)
  - financial DB queried by `as_of_date` (date-based snapshot/partitioning)
- LLM outputs are treated as structured data (JSON), never “free-form prose only”

## System Responsibilities

- Financial DB
  - ingest/update KRX-wide data (daily bars, flows; fundamentals if available)
  - support `as_of_date` queries (snapshot or partition strategy)
- Recommendation job (EOD)
  - build candidate universe (200~500) from internal filtering
  - call the LLM provider with a strict JSON schema requirement
  - persist the daily snapshot + per-stock entries

## EOD Scheduling (decision)

- Use GitHub Actions (cron) to trigger a one-shot worker run
- Avoid in-process schedulers to prevent duplicate runs during restarts/scale-out
- Dashboard
  - show today’s Top 20
  - show detail: DB metrics + AI rationale
  - browse/compare historical snapshots

## Implementation Direction (current)

- Backend language: Rust

## Data Contracts (recommended)

### Recommendation snapshot

- Identity: `as_of_date` (KR market date) + `generated_at`
- Content: Top 20 items with deterministic `rank`
- Storage: append-only; never mutate historical snapshots (create a new snapshot if rerun)

### LLM response shape

Agents should enforce a strict, parseable JSON response (provider-agnostic). Recommended fields:

```json
{
  "as_of_date": "YYYY-MM-DD",
  "generated_at": "ISO-8601",
  "items": [
    {
      "rank": 1,
      "ticker": "KRX:005930",
      "name": "삼성전자",
      "rationale": ["...", "...", "..."],
      "risk_notes": "...",
      "confidence": 0.0
    }
  ]
}
```

If the model returns invalid JSON, agents should:

- attempt a single repair pass (asking the model to re-emit valid JSON)
- otherwise fail the job safely (no partial writes, or mark snapshot as failed with diagnostics)

## LLM Provider Strategy

- Current provider: Anthropic
- Design requirement: keep the provider swappable (OpenAI is a future option)
- Implementation guidance
  - define a small internal interface (e.g., `generate_recommendations(input) -> SnapshotJson`)
  - isolate provider-specific request/response handling behind that interface
  - keep JSON schema enforcement and repair logic in one place

## Candidate Universe Rules

- Input to the LLM: 200~500 candidates after internal prefilter
- Liquidity filter: implemented as an internal option; default should match “KRX-wide” intent
- Candidate records passed to OpenAI should be strictly limited to:
  - as-of date
  - compact numeric features needed for ranking
  - avoid leaking any secrets or irrelevant data

## Monitoring / Health

- Sentry: capture backend job failures and UI errors
- UptimeRobot: `/healthz` endpoint should be cheap, deterministic, and not call OpenAI

## Security / Privacy

- Do not add auth unless explicitly requested
- Never commit secrets; use environment variables for:
  - `ANTHROPIC_API_KEY`
  - `OPENAI_API_KEY` (optional; for a future provider switch)
  - `DATABASE_URL` (recommended; Postgres connection string)
  - `SUPABASE_URL`, `SUPABASE_SERVICE_ROLE_KEY` (backend only; optional)
  - `SENTRY_DSN`

## Current Repo Layout (WIP)

- Workspace root: `Cargo.toml`
- Crates
  - `crates/core` (`tootoo_core`): shared types/config + provider abstraction
  - `crates/api` (`tootoo_api`): axum API server
  - `crates/worker` (`tootoo_worker`): one-shot EOD worker CLI

## How Agents Should Work Here

- Prefer additive, reversible changes (migrations + new code) over destructive edits
- Preserve append-only historical data; avoid UPDATE/DELETE on snapshots without explicit approval
- When adding schema: include `as_of_date` and stable identifiers to support reproducibility
- When adding new jobs: ensure idempotency (same day run should not corrupt prior snapshots)
