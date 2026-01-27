use crate::ingest::types::DailyFeatureItem;
use anyhow::Context;
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::Value;
use uuid::Uuid;

pub async fn upsert_daily_features_atomic(
    pool: &sqlx::PgPool,
    as_of_date: NaiveDate,
    items: &[DailyFeatureItem],
) -> anyhow::Result<u64> {
    anyhow::ensure!(!items.is_empty(), "items must be non-empty");

    let mut tx = pool.begin().await.context("begin transaction failed")?;

    let mut affected: u64 = 0;
    for item in items {
        let features = serde_json::to_value(&item.features).context("features serialize failed")?;
        let res = sqlx::query(
            "INSERT INTO stock_features_daily (as_of_date, ticker, name, trading_value, features) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (as_of_date, ticker) DO UPDATE \
               SET name = EXCLUDED.name, trading_value = EXCLUDED.trading_value, features = EXCLUDED.features",
        )
        .bind(as_of_date)
        .bind(item.ticker.trim())
        .bind(item.name.trim())
        .bind(item.trading_value)
        .bind(features)
        .execute(&mut *tx)
        .await
        .context("upsert stock_features_daily failed")?;

        affected += res.rows_affected();
    }

    tx.commit().await.context("commit transaction failed")?;
    Ok(affected)
}

pub async fn record_ingest_run(
    pool: &sqlx::PgPool,
    as_of_date: NaiveDate,
    provider: &str,
    status: &str,
    error: Option<&str>,
    raw_response: Option<Value>,
) -> anyhow::Result<Uuid> {
    let id = Uuid::new_v4();
    let generated_at: DateTime<Utc> = Utc::now();

    sqlx::query(
        "INSERT INTO stock_features_ingest_runs (id, as_of_date, generated_at, provider, status, error, raw_response) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(as_of_date)
    .bind(generated_at)
    .bind(provider)
    .bind(status)
    .bind(error)
    .bind(raw_response)
    .execute(pool)
    .await
    .context("insert stock_features_ingest_runs failed")?;

    Ok(id)
}
