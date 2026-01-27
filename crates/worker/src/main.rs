use clap::Parser;
use anyhow::Context;
use tracing_subscriber::EnvFilter;
use tootoo_core::llm::LlmClient;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod universe;

#[derive(Debug, Parser)]
#[command(name = "tootoo_worker")]
struct Args {
    /// Market as-of date (YYYY-MM-DD). Defaults to today's KST date for now.
    #[arg(long)]
    as_of_date: Option<String>,

    /// Do everything except writing to the database.
    #[arg(long)]
    dry_run: bool,
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

    let as_of_date = resolve_as_of_date(args.as_of_date.as_deref())?;

    let candidates = universe::build_candidate_universe_stub(as_of_date, universe::UniverseOptions::default())?;

    if args.dry_run {
        tracing::info!(
            %as_of_date,
            dry_run = true,
            candidates_len = candidates.len(),
            "worker placeholder: EOD run (dry-run)"
        );
        return Ok(());
    }

    let db_url = settings.require_database_url()?;

    let pool = sqlx::PgPoolOptions::new()
        .max_connections(5)
        .connect(db_url)
        .await
        .context("connect DATABASE_URL failed")?;

    tootoo_core::storage::migrate(&pool).await?;

    let acquired = tootoo_core::storage::lock::try_acquire_as_of_date_lock(&pool, as_of_date).await?;
    if !acquired {
        tracing::warn!(%as_of_date, "as_of_date lock not acquired; another run in progress")
;
        return Ok(());
    }

    let llm = tootoo_core::llm::anthropic::AnthropicClient::from_settings(&settings)?;
    let input = tootoo_core::llm::GenerateInput::try_new(as_of_date, candidates)?;

    let provider = "anthropic";
    let llm_result = llm.generate_recommendations(input).await;

    match llm_result {
        Ok(snapshot) => {
            let raw = serde_json::to_value(&snapshot).ok();
            let snapshot_id = tootoo_core::storage::recommendations::persist_success(
                &pool,
                &snapshot,
                provider,
                raw,
            )
            .await?;

            tracing::info!(%as_of_date, %snapshot_id, "persisted recommendation snapshot")
;
        }
        Err(err) => {
            sentry_anyhow::capture_anyhow(&err);
            let generated_at = chrono::Utc::now();
            let mut raw_llm_response: Option<serde_json::Value> = None;
            if let Some(diag) = err.downcast_ref::<tootoo_core::llm::error::LlmDiagnosticsError>() {
                if let Some(raw) = diag.raw_output.as_deref() {
                    raw_llm_response = serde_json::from_str(raw)
                        .ok()
                        .or_else(|| Some(serde_json::json!({"raw_text": raw}))); 
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

            tracing::error!(%as_of_date, %snapshot_id, error = %err, "recommendation run failed")
;
        }
    }

    let _ = tootoo_core::storage::lock::release_as_of_date_lock(&pool, as_of_date).await;
    Ok(())
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

fn resolve_as_of_date(as_of_date_arg: Option<&str>) -> anyhow::Result<chrono::NaiveDate> {
    if let Some(s) = as_of_date_arg {
        return Ok(chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")?);
    }

    // Default: KST date (UTC+9). This is a safe placeholder until KR market-date rules are
    // implemented (weekends/holidays/close-time cutoffs).
    let kst = chrono::FixedOffset::east_opt(9 * 3600).context("invalid KST offset")?;
    Ok(chrono::Utc::now().with_timezone(&kst).date_naive())
}
