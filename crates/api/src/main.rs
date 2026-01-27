use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::PgPool;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

use tootoo_core::domain::recommendation::{RecommendationItem, RecommendationSnapshot};

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
    let pool: Option<PgPool> = match settings.require_database_url() {
        Ok(db_url) => match sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .connect(db_url)
            .await
        {
            Ok(pool) => match tootoo_core::storage::migrate(&pool).await {
                Ok(()) => Some(pool),
                Err(e) => {
                    sentry_anyhow::capture_anyhow(&e);
                    tracing::error!(error = %e, "db migrations failed; starting API in degraded mode");
                    None
                }
            },
            Err(e) => {
                let err = anyhow::Error::new(e);
                sentry_anyhow::capture_anyhow(&err);
                tracing::error!(error = %err, "db connect failed; starting API in degraded mode");
                None
            }
        },
        Err(e) => {
            sentry_anyhow::capture_anyhow(&e);
            tracing::error!(error = %e, "DATABASE_URL missing; starting API in degraded mode");
            None
        }
    };

    let state = AppState { pool };

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/snapshots/latest", get(get_latest_snapshot))
        .route("/snapshots/:as_of_date", get(get_snapshot_by_date))
        .route(
            "/items/:as_of_date/:ticker",
            get(get_item_by_date_and_ticker),
        )
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3000);
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));

    tracing::info!(%addr, "api listening");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn healthz() -> &'static str {
    "ok"
}

#[derive(Debug, Clone)]
struct AppState {
    pool: Option<PgPool>,
}

#[derive(Debug, Serialize)]
struct ApiSnapshot {
    snapshot_id: Uuid,
    provider: String,
    snapshot: RecommendationSnapshot,
}

async fn get_latest_snapshot(
    State(state): State<AppState>,
) -> Result<Json<ApiSnapshot>, StatusCode> {
    let Some(pool) = &state.pool else {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    };

    let (snapshot_id, provider, snapshot) = fetch_snapshot(pool, None)
        .await
        .map_err(|e| {
            sentry_anyhow::capture_anyhow(&e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(ApiSnapshot {
        snapshot_id,
        provider,
        snapshot,
    }))
}

async fn get_snapshot_by_date(
    State(state): State<AppState>,
    Path(as_of_date): Path<String>,
) -> Result<Json<ApiSnapshot>, StatusCode> {
    let Some(pool) = &state.pool else {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    };

    let as_of_date =
        NaiveDate::parse_from_str(&as_of_date, "%Y-%m-%d").map_err(|_| StatusCode::BAD_REQUEST)?;

    let (snapshot_id, provider, snapshot) = fetch_snapshot(pool, Some(as_of_date))
        .await
        .map_err(|e| {
            sentry_anyhow::capture_anyhow(&e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(ApiSnapshot {
        snapshot_id,
        provider,
        snapshot,
    }))
}

async fn get_item_by_date_and_ticker(
    State(state): State<AppState>,
    Path((as_of_date, ticker)): Path<(String, String)>,
) -> Result<Json<RecommendationItem>, StatusCode> {
    let Some(pool) = &state.pool else {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    };

    let as_of_date =
        NaiveDate::parse_from_str(&as_of_date, "%Y-%m-%d").map_err(|_| StatusCode::BAD_REQUEST)?;

    let (snapshot_id, _, _) = fetch_snapshot(pool, Some(as_of_date))
        .await
        .map_err(|e| {
            sentry_anyhow::capture_anyhow(&e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    let item = fetch_item(pool, snapshot_id, &ticker)
        .await
        .map_err(|e| {
            sentry_anyhow::capture_anyhow(&e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(item))
}

async fn fetch_snapshot(
    pool: &PgPool,
    as_of_date: Option<NaiveDate>,
) -> anyhow::Result<Option<(Uuid, String, RecommendationSnapshot)>> {
    let row = match as_of_date {
        Some(d) => {
            sqlx::query_as::<_, (Uuid, NaiveDate, DateTime<Utc>, String)>(
                "SELECT id, as_of_date, generated_at, provider \
                 FROM recommendation_snapshots \
                 WHERE status = 'success' AND as_of_date = $1 \
                 ORDER BY generated_at DESC \
                 LIMIT 1",
            )
            .bind(d)
            .fetch_optional(pool)
            .await?
        }
        None => {
            sqlx::query_as::<_, (Uuid, NaiveDate, DateTime<Utc>, String)>(
                "SELECT id, as_of_date, generated_at, provider \
                 FROM recommendation_snapshots \
                 WHERE status = 'success' \
                 ORDER BY as_of_date DESC, generated_at DESC \
                 LIMIT 1",
            )
            .fetch_optional(pool)
            .await?
        }
    };

    let Some((id, as_of_date, generated_at, provider)) = row else {
        return Ok(None);
    };

    let items = fetch_items(pool, id).await?;

    Ok(Some((
        id,
        provider,
        RecommendationSnapshot {
            as_of_date,
            generated_at,
            items,
        },
    )))
}

async fn fetch_items(pool: &PgPool, snapshot_id: Uuid) -> anyhow::Result<Vec<RecommendationItem>> {
    let rows = sqlx::query_as::<
        _,
        (
            i32,
            String,
            String,
            Vec<String>,
            Option<String>,
            Option<f64>,
        ),
    >(
        "SELECT rank, ticker, name, rationale, risk_notes, confidence \
         FROM recommendation_items \
         WHERE snapshot_id = $1 \
         ORDER BY rank ASC",
    )
    .bind(snapshot_id)
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for (rank, ticker, name, rationale, risk_notes, confidence) in rows {
        anyhow::ensure!(
            rationale.len() == 3,
            "invalid rationale length in DB for snapshot_id={snapshot_id}, ticker={ticker}"
        );
        out.push(RecommendationItem {
            rank,
            ticker,
            name,
            rationale: [
                rationale[0].clone(),
                rationale[1].clone(),
                rationale[2].clone(),
            ],
            risk_notes,
            confidence,
        });
    }
    Ok(out)
}

async fn fetch_item(
    pool: &PgPool,
    snapshot_id: Uuid,
    ticker: &str,
) -> anyhow::Result<Option<RecommendationItem>> {
    let row = sqlx::query_as::<
        _,
        (
            i32,
            String,
            String,
            Vec<String>,
            Option<String>,
            Option<f64>,
        ),
    >(
        "SELECT rank, ticker, name, rationale, risk_notes, confidence \
         FROM recommendation_items \
         WHERE snapshot_id = $1 AND ticker = $2 \
         LIMIT 1",
    )
    .bind(snapshot_id)
    .bind(ticker)
    .fetch_optional(pool)
    .await?;

    let Some((rank, ticker, name, rationale, risk_notes, confidence)) = row else {
        return Ok(None);
    };

    if rationale.len() != 3 {
        return Ok(None);
    }

    Ok(Some(RecommendationItem {
        rank,
        ticker,
        name,
        rationale: [
            rationale[0].clone(),
            rationale[1].clone(),
            rationale[2].clone(),
        ],
        risk_notes,
        confidence,
    }))
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
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
