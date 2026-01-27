use anyhow::Context;
use chrono::{Datelike, NaiveDate};
use serde_json::json;

pub async fn ingest_stub_stock_features(
    pool: &sqlx::PgPool,
    as_of_date: NaiveDate,
    size: usize,
) -> anyhow::Result<u64> {
    anyhow::ensure!(
        (1..=5000).contains(&size),
        "ingest size must be 1..=5000 (got {size})"
    );

    let mut tx = pool.begin().await.context("begin transaction failed")?;

    let base = (as_of_date.num_days_from_ce() % 10_000) as f64;
    let mut inserted: u64 = 0;

    for i in 1..=size {
        let ticker = format!("KRX:{i:06}");
        let name = format!("Stub {i:06}");
        let trading_value = ((size - i + 1) as f64) * 1.0e8;

        // Compact numeric features only.
        let features = json!({
            "ret_1d": ((i as f64) % 200.0 - 100.0) / 1000.0,
            "mom_5d": (base + (i as f64)) / 1000.0,
            "vol_20d": ((i as f64) % 50.0) / 100.0,
            "value_score": ((size - i + 1) as f64) / (size as f64),
        });

        let res = sqlx::query(
            "INSERT INTO stock_features_daily (as_of_date, ticker, name, trading_value, features) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (as_of_date, ticker) DO NOTHING",
        )
        .bind(as_of_date)
        .bind(ticker)
        .bind(name)
        .bind(trading_value)
        .bind(features)
        .execute(&mut *tx)
        .await
        .context("insert stock_features_daily failed")?;

        inserted += res.rows_affected();
    }

    tx.commit().await.context("commit transaction failed")?;
    Ok(inserted)
}
