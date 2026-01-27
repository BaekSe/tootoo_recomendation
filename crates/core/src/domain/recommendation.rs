use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecommendationSnapshot {
    pub as_of_date: NaiveDate,
    pub generated_at: DateTime<Utc>,
    pub items: Vec<RecommendationItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecommendationItem {
    pub rank: i32,
    pub ticker: String,
    pub name: String,
    pub rationale: [String; 3],
    pub risk_notes: Option<String>,
    pub confidence: Option<f64>,
}
