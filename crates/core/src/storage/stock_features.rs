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

    // Batch the upsert to reduce round trips (critical for CI runners / remote DB).
    // Keep it transactional.
    let mut affected: u64 = 0;
    let chunk_size: usize = std::env::var("STOCK_FEATURES_UPSERT_BATCH")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(200);

    anyhow::ensure!(chunk_size >= 1, "STOCK_FEATURES_UPSERT_BATCH must be >= 1");

    let mut batch_idx: usize = 0;
    for chunk in items.chunks(chunk_size) {
        batch_idx += 1;
        let t0 = std::time::Instant::now();
        let mut qb = sqlx::QueryBuilder::new(
            "INSERT INTO stock_features_daily (as_of_date, ticker, name, trading_value, features) ",
        );
        qb.push_values(chunk, |mut b, item| {
            // This should not fail because features are numeric-only (enforced upstream).
            let features = serde_json::to_value(&item.features).expect("features serialize failed");
            b.push_bind(as_of_date)
                .push_bind(item.ticker.trim())
                .push_bind(item.name.trim())
                .push_bind(item.trading_value)
                .push_bind(features);
        });
        qb.push(
            " ON CONFLICT (as_of_date, ticker) DO UPDATE \
               SET name = EXCLUDED.name, trading_value = EXCLUDED.trading_value, features = EXCLUDED.features",
        );

        let res = qb
            .build()
            .persistent(false)
            .execute(&mut *tx)
            .await
            .context("batch upsert stock_features_daily failed")?;
        affected += res.rows_affected();

        tracing::debug!(
            %as_of_date,
            batch_idx,
            batch_size = chunk.len(),
            elapsed_ms = t0.elapsed().as_millis(),
            "stock_features_daily batch upsert"
        );
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
    .persistent(false)
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
