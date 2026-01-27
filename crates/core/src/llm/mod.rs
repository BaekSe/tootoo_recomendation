use crate::domain::recommendation::{Candidate, RecommendationSnapshot};

pub mod anthropic;
pub mod error;
pub mod json;

#[derive(Debug, Clone)]
pub struct GenerateInput {
    pub as_of_date: chrono::NaiveDate,
    pub candidates: Vec<Candidate>,
}

impl GenerateInput {
    pub const MIN_CANDIDATES: usize = 200;
    pub const MAX_CANDIDATES: usize = 500;

    pub fn try_new(
        as_of_date: chrono::NaiveDate,
        candidates: Vec<Candidate>,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            (Self::MIN_CANDIDATES..=Self::MAX_CANDIDATES).contains(&candidates.len()),
            "candidate universe must be {}..={} (got {})",
            Self::MIN_CANDIDATES,
            Self::MAX_CANDIDATES,
            candidates.len()
        );

        Ok(Self {
            as_of_date,
            candidates,
        })
    }

    pub fn candidates_json(&self) -> serde_json::Value {
        serde_json::json!({
            "as_of_date": self.as_of_date,
            "candidates": self.candidates,
        })
    }
}

#[derive(Debug, Clone)]
pub enum Provider {
    Anthropic,
    OpenAI,
}

#[async_trait::async_trait]
pub trait LlmClient: Send + Sync {
    fn provider(&self) -> Provider;

    async fn generate_recommendations(
        &self,
        input: GenerateInput,
    ) -> anyhow::Result<RecommendationSnapshot>;
}
