use crate::domain::recommendation::RecommendationSnapshot;

#[derive(Debug, Clone)]
pub struct GenerateInput {
    pub as_of_date: chrono::NaiveDate,
    pub candidates_json: serde_json::Value,
}

#[derive(Debug, Clone)]
pub enum Provider {
    Anthropic,
    OpenAI,
}

#[async_trait::async_trait]
pub trait LlmClient: Send + Sync {
    fn provider(&self) -> Provider;

    async fn generate_recommendations(&self, input: GenerateInput)
        -> anyhow::Result<RecommendationSnapshot>;
}
