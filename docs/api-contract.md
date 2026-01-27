# API Contract (Dashboard)

Base URL: API server

## Health

`GET /healthz`

Response (200):

```text
ok
```

## Latest Snapshot

`GET /snapshots/latest`

Response (200):

```json
{
  "snapshot_id": "uuid",
  "provider": "anthropic",
  "snapshot": {
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
}
```

Response (404): no successful snapshots

## Snapshot By Date

`GET /snapshots/:as_of_date`

- `:as_of_date` format: `YYYY-MM-DD`

Response: same shape as `/snapshots/latest`

## Item Detail

`GET /items/:as_of_date/:ticker`

- `:as_of_date` format: `YYYY-MM-DD`
- `:ticker` format: provider-agnostic string (e.g., `KRX:005930`)

Response (200):

```json
{
  "rank": 1,
  "ticker": "KRX:005930",
  "name": "삼성전자",
  "rationale": ["...", "...", "..."],
  "risk_notes": "...",
  "confidence": 0.0
}
```
