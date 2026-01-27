use anyhow::Context;
use chrono::{Datelike, NaiveDate};

// Advisory locks are scoped to the Postgres session. This is used as a best-effort guard against
// concurrent EOD runs for the same as-of date.
const LOCK_NAMESPACE: i64 = 0x544F_4F54_4F4F; // "TOOTOO" as hex-ish namespace.

fn lock_key_for_date(as_of_date: NaiveDate) -> i64 {
    LOCK_NAMESPACE ^ (as_of_date.num_days_from_ce() as i64)
}

pub async fn try_acquire_as_of_date_lock(
    pool: &sqlx::PgPool,
    as_of_date: NaiveDate,
) -> anyhow::Result<bool> {
    let mut conn = pool
        .acquire()
        .await
        .context("acquire connection for advisory lock failed")?;
    try_acquire_as_of_date_lock_conn(&mut *conn, as_of_date).await
}

pub async fn try_acquire_as_of_date_lock_conn(
    conn: &mut sqlx::PgConnection,
    as_of_date: NaiveDate,
) -> anyhow::Result<bool> {
    let key = lock_key_for_date(as_of_date);
    let acquired: (bool,) = sqlx::query_as("SELECT pg_try_advisory_lock($1)")
        .persistent(false)
        .bind(key)
        .fetch_one(conn)
        .await
        .with_context(|| format!("failed to acquire advisory lock (key={key})"))?;
    Ok(acquired.0)
}

pub async fn release_as_of_date_lock(
    pool: &sqlx::PgPool,
    as_of_date: NaiveDate,
) -> anyhow::Result<()> {
    let mut conn = pool
        .acquire()
        .await
        .context("acquire connection for advisory unlock failed")?;
    release_as_of_date_lock_conn(&mut *conn, as_of_date).await
}

pub async fn release_as_of_date_lock_conn(
    conn: &mut sqlx::PgConnection,
    as_of_date: NaiveDate,
) -> anyhow::Result<()> {
    let key = lock_key_for_date(as_of_date);
    sqlx::query("SELECT pg_advisory_unlock($1)")
        .persistent(false)
        .bind(key)
        .execute(conn)
        .await
        .with_context(|| format!("failed to release advisory lock (key={key})"))?;
    Ok(())
}
