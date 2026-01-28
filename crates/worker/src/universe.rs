use chrono::{Datelike, NaiveDate};
use std::collections::BTreeMap;
use tootoo_core::domain::recommendation::Candidate;

#[derive(Debug, Clone)]
pub struct UniverseOptions {
    /// Number of candidates to pass to the LLM (must be 200..=500).
    pub size: usize,

    /// Optional placeholder for a future liquidity filter.
    pub min_trading_value: Option<f64>,

    /// Oversampling factor for the initial liquidity screen.
    /// We fetch (size * oversample) rows by trading value, then rescore and select top `size`.
    pub oversample: usize,
}

impl Default for UniverseOptions {
    fn default() -> Self {
        Self {
            size: 200,
            min_trading_value: None,
            oversample: 5,
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

        if let Ok(s) = std::env::var("UNIVERSE_OVERSAMPLE") {
            if let Ok(n) = s.parse::<usize>() {
                out.oversample = n;
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

    anyhow::ensure!(opts.oversample >= 1, "UNIVERSE_OVERSAMPLE must be >= 1");
    let limit = (opts.size.saturating_mul(opts.oversample)).max(opts.size);

    let rows = match opts.min_trading_value {
        Some(min_tv) => {
            sqlx::query_as::<_, (String, String, serde_json::Value, Option<f64>)>(
                "SELECT ticker, name, features, trading_value \
                 FROM stock_features_daily \
                 WHERE as_of_date = $1 AND trading_value IS NOT NULL AND trading_value >= $2 \
                 ORDER BY trading_value DESC NULLS LAST, ticker ASC \
                 LIMIT $3",
            )
            .persistent(false)
            .bind(as_of_date)
            .bind(min_tv)
            .bind(limit as i64)
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query_as::<_, (String, String, serde_json::Value, Option<f64>)>(
                "SELECT ticker, name, features, trading_value \
                 FROM stock_features_daily \
                 WHERE as_of_date = $1 \
                 ORDER BY trading_value DESC NULLS LAST, ticker ASC \
                 LIMIT $2",
            )
            .persistent(false)
            .bind(as_of_date)
            .bind(limit as i64)
            .fetch_all(pool)
            .await?
        }
    };

    // Filter out ETFs/ETNs (we only want single-name equities).
    // KIS master does not currently provide an explicit instrument type, so use a conservative
    // name-based heuristic.
    let rows: Vec<_> = rows
        .into_iter()
        .filter(|(_ticker, name, _features, _tv)| !is_etf_or_etn_name(name))
        .collect();

    anyhow::ensure!(
        rows.len() >= opts.size,
        "insufficient candidates for as_of_date={as_of_date} after ETF/ETN exclusion: expected at least {}, got {}",
        opts.size,
        rows.len()
    );

    // Score candidates: liquidity dominates (trading_value), then a small 1d return tilt.
    let mut scored: Vec<(f64, Candidate)> = Vec::with_capacity(rows.len());
    for (ticker, name, features_json, trading_value) in rows {
        let features = json_to_feature_map(features_json);
        let tv = trading_value.unwrap_or(0.0);
        let ret_1d = features.get("ret_1d").copied().unwrap_or(0.0);

        // trading_value can be huge; scale to billions KRW-ish units.
        let score = (tv / 1_000_000_000.0) + (ret_1d * 10.0);

        scored.push((
            score,
            Candidate {
                ticker,
                name,
                features,
            },
        ));
    }

    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.ticker.cmp(&b.1.ticker))
    });

    let mut out = Vec::with_capacity(opts.size);
    for (_, c) in scored.into_iter().take(opts.size) {
        out.push(c);
    }

    Ok(out)
}

fn is_etf_or_etn_name(name: &str) -> bool {
    let s = name.trim();
    if s.is_empty() {
        return false;
    }

    // Common Korean ETF/ETN markers.
    // Keep this conservative: exclude obvious passive products.
    let lower = s.to_ascii_lowercase();
    if lower.contains("etf") || lower.contains("etn") {
        return true;
    }

    // Korean keywords often present in ETF names.
    // NOTE: We intentionally do NOT exclude generic words like "코스닥" (can appear in company names)
    // without ETF-like wrappers.
    s.contains("KODEX")
        || s.contains("TIGER")
        || s.contains("KOSEF")
        || s.contains("KBSTAR")
        || s.contains("ARIRANG")
        || s.contains("HANARO")
        || s.contains("SOL")
        || s.contains("ACE")
        || s.contains("TIMEFOLIO")
        || s.contains("PLUS")
        || s.contains("1Q")
        || s.contains("RISE")
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rescoring_prefers_ret_1d_given_equal_trading_value() {
        // Reuse the scoring logic via a tiny local helper to keep the test focused.
        fn score(tv: f64, ret_1d: f64) -> f64 {
            (tv / 1_000_000_000.0) + (ret_1d * 10.0)
        }

        let tv = 1_000_000_000.0;
        let a = (
            score(tv, 0.02),
            Candidate {
                ticker: "KRX:000001".to_string(),
                name: "A".to_string(),
                features: json_to_feature_map(json!({"ret_1d": 0.02})),
            },
        );
        let b = (
            score(tv, -0.01),
            Candidate {
                ticker: "KRX:000002".to_string(),
                name: "B".to_string(),
                features: json_to_feature_map(json!({"ret_1d": -0.01})),
            },
        );
        let mut scored = vec![b, a];
        scored.sort_by(|x, y| {
            y.0.partial_cmp(&x.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| x.1.ticker.cmp(&y.1.ticker))
        });

        let out: Vec<Candidate> = scored.into_iter().take(2).map(|(_, c)| c).collect();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].ticker, "KRX:000001");
        assert_eq!(out[1].ticker, "KRX:000002");
    }

    #[test]
    fn excludes_obvious_etf_names() {
        assert!(is_etf_or_etn_name("KODEX 코스닥150레버리지"));
        assert!(is_etf_or_etn_name("TIGER 미국S&P500"));
        assert!(is_etf_or_etn_name("Foo ETF"));
        assert!(is_etf_or_etn_name("Bar ETN"));
        assert!(!is_etf_or_etn_name("삼성전자"));
    }
}
