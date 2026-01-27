use chrono::{Datelike, NaiveDate};
use std::collections::BTreeMap;
use tootoo_core::domain::recommendation::Candidate;

#[derive(Debug, Clone)]
pub struct UniverseOptions {
    /// Number of candidates to pass to the LLM (must be 200..=500).
    pub size: usize,

    /// Optional placeholder for a future liquidity filter.
    pub min_trading_value: Option<f64>,
}

impl Default for UniverseOptions {
    fn default() -> Self {
        Self {
            size: 200,
            min_trading_value: None,
        }
    }
}

impl UniverseOptions {
    pub fn from_env() -> Self {
        let mut out = Self::default();

        if let Ok(s) = std::env::var("UNIVERSE_SIZE") {
            if let Ok(n) = s.parse::<usize>() {
                out.size = n;
            }
        }

        if let Ok(s) = std::env::var("UNIVERSE_MIN_TRADING_VALUE") {
            if let Ok(n) = s.parse::<f64>() {
                out.min_trading_value = Some(n);
            }
        }

        out
    }
}

pub fn build_candidate_universe_stub(
    as_of_date: NaiveDate,
    opts: UniverseOptions,
) -> anyhow::Result<Vec<Candidate>> {
    anyhow::ensure!(
        (200..=500).contains(&opts.size),
        "candidate universe size must be 200..=500 (got {})",
        opts.size
    );

    // Deterministic placeholder universe.
    // Replace with real KRX-wide ingestion + prefilter, queried as-of-date.
    let mut out = Vec::with_capacity(opts.size);
    for i in 1..=opts.size {
        let mut features = BTreeMap::new();
        features.insert(
            "stub_feature".to_string(),
            (as_of_date.num_days_from_ce() as f64) + (i as f64),
        );
        if let Some(v) = opts.min_trading_value {
            features.insert("min_trading_value".to_string(), v);
        }

        out.push(Candidate {
            ticker: format!("KRX:{i:06}"),
            name: format!("Stub {i:06}"),
            features,
        });
    }

    Ok(out)
}

pub async fn build_candidate_universe_db(
    pool: &sqlx::PgPool,
    as_of_date: NaiveDate,
    opts: UniverseOptions,
) -> anyhow::Result<Vec<Candidate>> {
    anyhow::ensure!(
        (200..=500).contains(&opts.size),
        "candidate universe size must be 200..=500 (got {})",
        opts.size
    );

    let rows = match opts.min_trading_value {
        Some(min_tv) => {
            sqlx::query_as::<_, (String, String, serde_json::Value)>(
                "SELECT ticker, name, features \
                 FROM stock_features_daily \
                 WHERE as_of_date = $1 AND trading_value IS NOT NULL AND trading_value >= $2 \
                 ORDER BY trading_value DESC NULLS LAST, ticker ASC \
                 LIMIT $3",
            )
            .bind(as_of_date)
            .bind(min_tv)
            .bind(opts.size as i64)
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query_as::<_, (String, String, serde_json::Value)>(
                "SELECT ticker, name, features \
                 FROM stock_features_daily \
                 WHERE as_of_date = $1 \
                 ORDER BY trading_value DESC NULLS LAST, ticker ASC \
                 LIMIT $2",
            )
            .bind(as_of_date)
            .bind(opts.size as i64)
            .fetch_all(pool)
            .await?
        }
    };

    anyhow::ensure!(
        rows.len() == opts.size,
        "insufficient candidates for as_of_date={as_of_date}: expected {}, got {}",
        opts.size,
        rows.len()
    );

    let mut out = Vec::with_capacity(rows.len());
    for (ticker, name, features_json) in rows {
        out.push(Candidate {
            ticker,
            name,
            features: json_to_feature_map(features_json),
        });
    }

    Ok(out)
}

fn json_to_feature_map(v: serde_json::Value) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    let obj = match v {
        serde_json::Value::Object(o) => o,
        _ => return out,
    };

    for (k, val) in obj {
        if let Some(n) = val.as_f64() {
            out.insert(k, n);
        }
    }

    out
}
