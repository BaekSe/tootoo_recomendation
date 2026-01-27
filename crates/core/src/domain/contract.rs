use crate::domain::recommendation::{RecommendationItem, RecommendationSnapshot};
use anyhow::{bail, ensure};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRecommendationSnapshot {
    pub as_of_date: NaiveDate,
    pub generated_at: DateTime<Utc>,
    pub items: Vec<LlmRecommendationItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRecommendationItem {
    pub rank: i32,
    pub ticker: String,
    pub name: String,
    pub rationale: Vec<String>,
    pub risk_notes: Option<String>,
    pub confidence: Option<f64>,
}

impl LlmRecommendationSnapshot {
    pub fn validate_and_into_snapshot(
        self,
        expected_as_of_date: NaiveDate,
    ) -> anyhow::Result<RecommendationSnapshot> {
        ensure!(
            self.as_of_date == expected_as_of_date,
            "LLM output as_of_date mismatch: expected {expected_as_of_date}, got {}",
            self.as_of_date
        );

        ensure!(
            self.items.len() == 20,
            "LLM output must contain exactly 20 items (got {})",
            self.items.len()
        );

        let mut seen_ranks = BTreeSet::<i32>::new();
        let mut items = Vec::with_capacity(self.items.len());
        for item in self.items {
            items.push(item.validate_and_into_item(&mut seen_ranks)?);
        }

        // Ensure ranks are contiguous 1..=20.
        for rank in 1..=20 {
            if !seen_ranks.contains(&rank) {
                bail!("missing rank {rank} in LLM output");
            }
        }

        Ok(RecommendationSnapshot {
            as_of_date: self.as_of_date,
            generated_at: self.generated_at,
            items,
        })
    }
}

impl LlmRecommendationItem {
    fn validate_and_into_item(
        self,
        seen_ranks: &mut BTreeSet<i32>,
    ) -> anyhow::Result<RecommendationItem> {
        ensure!(
            (1..=20).contains(&self.rank),
            "rank out of range: {}",
            self.rank
        );
        ensure!(
            seen_ranks.insert(self.rank),
            "duplicate rank: {}",
            self.rank
        );

        let ticker = self.ticker.trim().to_string();
        ensure!(!ticker.is_empty(), "ticker must be non-empty");

        let name = self.name.trim().to_string();
        ensure!(!name.is_empty(), "name must be non-empty");

        ensure!(
            self.rationale.len() == 3,
            "rationale must have exactly 3 lines (got {})",
            self.rationale.len()
        );
        let r0 = self.rationale[0].trim().to_string();
        let r1 = self.rationale[1].trim().to_string();
        let r2 = self.rationale[2].trim().to_string();
        ensure!(
            !r0.is_empty() && !r1.is_empty() && !r2.is_empty(),
            "rationale lines must be non-empty"
        );

        if let Some(confidence) = self.confidence {
            ensure!(
                (0.0..=1.0).contains(&confidence),
                "confidence must be between 0 and 1 (got {confidence})"
            );
        }

        let risk_notes = self
            .risk_notes
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        Ok(RecommendationItem {
            rank: self.rank,
            ticker,
            name,
            rationale: [r0, r1, r2],
            risk_notes,
            confidence: self.confidence,
        })
    }
}
