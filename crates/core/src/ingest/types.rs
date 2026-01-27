use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyFeaturesResponse {
    pub as_of_date: NaiveDate,
    pub items: Vec<DailyFeatureItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyFeatureItem {
    pub ticker: String,
    pub name: String,
    pub trading_value: Option<f64>,
    pub features: BTreeMap<String, f64>,
}
