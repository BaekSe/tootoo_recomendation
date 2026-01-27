use crate::domain::recommendation::{RecommendationItem, RecommendationSnapshot};
use anyhow::Context;

pub async fn persist_success(
    pool: &sqlx::PgPool,
    snapshot: &RecommendationSnapshot,
    provider: &str,
    raw_llm_response: Option<serde_json::Value>,
) -> anyhow::Result<uuid::Uuid> {
    anyhow::ensure!(
        snapshot.items.len() == 20,
        "snapshot must have exactly 20 items"
    );

    let mut tx = pool.begin().await.context("begin transaction failed")?;

    let snapshot_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO recommendation_snapshots (as_of_date, generated_at, provider, status, error, raw_llm_response) \
         VALUES ($1, $2, $3, 'success', NULL, $4) \
         RETURNING id",
    )
    .bind(snapshot.as_of_date)
    .bind(snapshot.generated_at)
    .bind(provider)
    .bind(raw_llm_response)
    .fetch_one(&mut *tx)
    .await
    .context("insert recommendation_snapshots failed")?;

    for item in &snapshot.items {
        insert_item(&mut tx, snapshot_id, item).await?;
    }

    tx.commit().await.context("commit transaction failed")?;
    Ok(snapshot_id)
}

pub async fn persist_failure(
    pool: &sqlx::PgPool,
    as_of_date: chrono::NaiveDate,
    generated_at: chrono::DateTime<chrono::Utc>,
    provider: &str,
    error: &str,
    raw_llm_response: Option<serde_json::Value>,
) -> anyhow::Result<uuid::Uuid> {
    let snapshot_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO recommendation_snapshots (as_of_date, generated_at, provider, status, error, raw_llm_response) \
         VALUES ($1, $2, $3, 'error', $4, $5) \
         RETURNING id",
    )
    .bind(as_of_date)
    .bind(generated_at)
    .bind(provider)
    .bind(error)
    .bind(raw_llm_response)
    .fetch_one(pool)
    .await
    .context("insert error recommendation_snapshots failed")?;

    Ok(snapshot_id)
}

async fn insert_item(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    snapshot_id: uuid::Uuid,
    item: &RecommendationItem,
) -> anyhow::Result<()> {
    let rationale: Vec<String> = item.rationale.iter().cloned().collect();

    sqlx::query(
        "INSERT INTO recommendation_items (snapshot_id, rank, ticker, name, rationale, risk_notes, confidence) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(snapshot_id)
    .bind(item.rank)
    .bind(&item.ticker)
    .bind(&item.name)
    .bind(rationale)
    .bind(&item.risk_notes)
    .bind(item.confidence)
    .execute(&mut **tx)
    .await
    .context("insert recommendation_items failed")?;

    Ok(())
}
