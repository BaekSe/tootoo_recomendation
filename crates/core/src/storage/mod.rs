use anyhow::Context;

pub mod lock;
pub mod recommendations;
pub mod stock_features;

pub async fn migrate(pool: &sqlx::PgPool) -> anyhow::Result<()> {
    // For Supabase connection pooler, prepared statements can be unsafe.
    // `sqlx::migrate!` uses prepared statements internally; use the executor API which
    // runs raw SQL strings.
    let migrator = sqlx::migrate!("./migrations");
    let mut conn = pool
        .acquire()
        .await
        .context("acquire connection for migrations failed")?;
    migrator
        .run_direct(&mut *conn)
        .await
        .context("sqlx migrations failed")?;
    Ok(())
}
