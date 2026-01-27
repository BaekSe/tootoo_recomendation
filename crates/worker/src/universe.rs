use chrono::NaiveDate;
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

pub fn build_candidate_universe_stub(as_of_date: NaiveDate, opts: UniverseOptions) -> anyhow::Result<Vec<Candidate>> {
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
        features.insert("stub_feature".to_string(), (as_of_date.num_days_from_ce() as f64) + (i as f64));
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
