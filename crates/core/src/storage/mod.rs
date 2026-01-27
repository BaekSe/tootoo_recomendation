use anyhow::Context;

pub mod lock;
pub mod recommendations;
pub mod stock_features;

pub async fn migrate(pool: &sqlx::PgPool) -> anyhow::Result<()> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .context("sqlx migrations failed")?;
    Ok(())
}
