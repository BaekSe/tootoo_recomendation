use anyhow::Context;
use clap::Parser;
use sqlx::postgres::PgConnectOptions;
use std::str::FromStr;
use tootoo_core::ingest::provider::DataProviderClient;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod ingest;
mod universe;

#[derive(Debug, Parser)]
#[command(name = "tootoo_worker")]
struct Args {
    /// Market as-of date (YYYY-MM-DD). Defaults to resolved KR market date (KST, close cutoff).
    #[arg(long)]
    as_of_date: Option<String>,

    /// Do everything except writing to the database.
    #[arg(long)]
    dry_run: bool,

    /// Seed stock_features_daily with deterministic stub rows for the resolved as_of_date.
    #[arg(long)]
    ingest_features: bool,

    /// Fetch stock_features_daily from an external data provider and upsert into DB.
    #[arg(long)]
    ingest_external: bool,

    /// Number of stub rows to insert when using --ingest-features.
    #[arg(long)]
    ingest_size: Option<usize>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    let settings = tootoo_core::config::Settings::from_env()?;
    let _sentry_guard = init_sentry(&settings);

    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::layer())
        .with(sentry_tracing::layer())
        .init();

    let args = Args::parse();

    let as_of_date = tootoo_core::time::kr_market::resolve_as_of_date(
        args.as_of_date.as_deref(),
        chrono::Utc::now(),
    )?;

    if args.dry_run {
        tracing::info!(
            %as_of_date,
            dry_run = true,
            "worker: EOD run (dry-run)"
        );
        return Ok(());
    }

    // Allow a worker-only override so we can bypass Supabase pooler if needed.
    let db_url = match std::env::var("WORKER_DATABASE_URL") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => settings.require_database_url()?.to_string(),
    };

    let mut connect_options =
        PgConnectOptions::from_str(&db_url).context("parse DATABASE_URL failed")?;
    connect_options = connect_options.statement_cache_capacity(0);

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect_with(connect_options)
        .await
        .context("connect DATABASE_URL failed")?;

    tootoo_core::storage::migrate(&pool).await?;

    if args.ingest_features {
        let size = args.ingest_size.unwrap_or(500);
        let inserted = ingest::ingest_stub_stock_features(&pool, as_of_date, size).await?;
        tracing::info!(%as_of_date, size, inserted, "seeded stock_features_daily (stub)");
        return Ok(());
    }

    if args.ingest_external {
        let provider =
            tootoo_core::ingest::provider::HttpJsonDataProvider::from_settings(&settings)?;
        let provider_name = provider.provider_name();

        let fetched = provider.fetch_daily_features(as_of_date).await;
        match fetched {
            Ok((resp, raw_json)) => {
                let affected = tootoo_core::storage::stock_features::upsert_daily_features_atomic(
                    &pool,
                    as_of_date,
                    &resp.items,
                )
                .await?;

                let run_id = tootoo_core::storage::stock_features::record_ingest_run(
                    &pool,
                    as_of_date,
                    provider_name,
                    "success",
                    None,
                    Some(raw_json),
                )
                .await?;

                tracing::info!(%as_of_date, %run_id, affected, items = resp.items.len(), "external ingest complete");
                return Ok(());
            }
            Err(err) => {
                sentry_anyhow::capture_anyhow(&err);
                let run_id = tootoo_core::storage::stock_features::record_ingest_run(
                    &pool,
                    as_of_date,
                    provider_name,
                    "error",
                    Some(&format!("{:#}", err)),
                    None,
                )
                .await?;

                tracing::error!(%as_of_date, %run_id, error = %err, "external ingest failed");
                return Err(err);
            }
        }
    }

    // Advisory locks are session-scoped, so we must acquire and release on the same connection.
    let mut lock_conn = pool
        .acquire()
        .await
        .context("acquire connection for advisory lock failed")?;
    let acquired =
        tootoo_core::storage::lock::try_acquire_as_of_date_lock_conn(&mut *lock_conn, as_of_date)
            .await?;
    if !acquired {
        tracing::warn!(%as_of_date, "as_of_date lock not acquired; another run in progress");
        return Ok(());
    }

    if success_snapshot_exists(&pool, as_of_date).await? {
        tracing::info!(%as_of_date, "successful snapshot already exists; exiting (no-op)");
        let _ =
            tootoo_core::storage::lock::release_as_of_date_lock_conn(&mut *lock_conn, as_of_date)
                .await;
        return Ok(());
    }

    let universe_opts = universe::UniverseOptions::from_env();
    let use_stub = std::env::var("TOOTOO_USE_STUB_UNIVERSE").ok().is_some();
    let candidates = if use_stub {
        universe::build_candidate_universe_stub(as_of_date, universe_opts)?
    } else {
        universe::build_candidate_universe_db(&pool, as_of_date, universe_opts).await?
    };

    let llm = tootoo_core::llm::anthropic::AnthropicClient::from_settings(&settings)?;
    let input = tootoo_core::llm::GenerateInput::try_new(as_of_date, candidates)?;

    let provider = "anthropic";
    let llm_result = llm.generate_recommendations_with_raw(input).await;

    match llm_result {
        Ok((snapshot, raw_json)) => {
            match tootoo_core::storage::recommendations::persist_success(
                &pool,
                &snapshot,
                provider,
                Some(raw_json),
            )
            .await
            {
                Ok(snapshot_id) => {
                    tracing::info!(%as_of_date, %snapshot_id, "persisted recommendation snapshot");
                }
                Err(e) => {
                    if is_unique_violation(&e) {
                        tracing::info!(%as_of_date, "snapshot already exists (unique constraint); treating as no-op");
                    } else {
                        let generated_at = chrono::Utc::now();
                        let _ = tootoo_core::storage::recommendations::persist_failure(
                            &pool,
                            as_of_date,
                            generated_at,
                            provider,
                            &format!("persist_success failed: {:#}", e),
                            None,
                        )
                        .await;

                        tracing::error!(%as_of_date, error = %e, "persist_success failed");
                    }
                }
            }
        }
        Err(err) => {
            sentry_anyhow::capture_anyhow(&err);
            let generated_at = chrono::Utc::now();
            let mut raw_llm_response: Option<serde_json::Value> = None;
            if let Some(diag) = err.downcast_ref::<tootoo_core::llm::error::LlmDiagnosticsError>() {
                raw_llm_response = diag.raw_response_json.clone();
                if raw_llm_response.is_none() {
                    if let Some(raw) = diag.raw_output.as_deref() {
                        raw_llm_response = serde_json::from_str(raw)
                            .ok()
                            .or_else(|| Some(serde_json::json!({"raw_text": raw})));
                    }
                }
            }

            let snapshot_id = tootoo_core::storage::recommendations::persist_failure(
                &pool,
                as_of_date,
                generated_at,
                provider,
                &format!("{:#}", err),
                raw_llm_response,
            )
            .await?;

            tracing::error!(%as_of_date, %snapshot_id, error = %err, "recommendation run failed");
        }
    }

    let _ =
        tootoo_core::storage::lock::release_as_of_date_lock_conn(&mut *lock_conn, as_of_date).await;
    Ok(())
}

fn is_unique_violation(err: &anyhow::Error) -> bool {
    let Some(sqlx_err) = err.downcast_ref::<sqlx::Error>() else {
        return false;
    };

    let sqlx::Error::Database(db) = sqlx_err else {
        return false;
    };

    db.code().as_deref() == Some("23505")
}

fn init_sentry(settings: &tootoo_core::config::Settings) -> Option<sentry::ClientInitGuard> {
    let dsn = settings.sentry_dsn.as_deref()?;
    Some(sentry::init((
        dsn,
        sentry::ClientOptions {
            release: sentry::release_name!(),
            ..Default::default()
        },
    )))
}

async fn success_snapshot_exists(
    pool: &sqlx::PgPool,
    as_of_date: chrono::NaiveDate,
) -> anyhow::Result<bool> {
    let exists: Option<(i32,)> = sqlx::query_as(
        "SELECT 1 FROM recommendation_snapshots WHERE status = 'success' AND as_of_date = $1 LIMIT 1",
    )
    .persistent(false)
    .bind(as_of_date)
    .fetch_optional(pool)
    .await?;
    Ok(exists.is_some())
}
