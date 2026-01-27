use anyhow::Context;

pub async fn migrate(pool: &sqlx::PgPool) -> anyhow::Result<()> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .context("sqlx migrations failed")?;
    Ok(())
}
