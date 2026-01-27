use crate::config::Settings;
use crate::ingest::types::{DailyFeatureItem, DailyFeaturesResponse};
use anyhow::{Context, Result};
use chrono::NaiveDate;
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::Value;
use std::time::Duration;

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const DEFAULT_PATH: &str = "/v1/stock_features_daily";
const DEFAULT_RETRIES: u32 = 3;

#[async_trait::async_trait]
pub trait DataProviderClient: Send + Sync {
    fn provider_name(&self) -> &'static str;

    async fn fetch_daily_features(
        &self,
        as_of_date: NaiveDate,
    ) -> Result<(DailyFeaturesResponse, Value)>;
}

#[derive(Debug, Clone)]
pub struct HttpJsonDataProvider {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    path: String,
    retries: u32,
}

impl HttpJsonDataProvider {
    pub fn from_settings(settings: &Settings) -> Result<Self> {
        let base_url = settings.require_data_provider_base_url()?.to_string();
        let api_key = settings.data_provider_api_key.clone();

        let timeout_secs = std::env::var("DATA_PROVIDER_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_TIMEOUT_SECS);

        let retries = std::env::var("DATA_PROVIDER_RETRIES")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(DEFAULT_RETRIES);

        let path = std::env::var("DATA_PROVIDER_FEATURES_PATH")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_PATH.to_string());

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .context("failed to build data provider http client")?;

        Ok(Self {
            http,
            base_url,
            api_key,
            path,
            retries,
        })
    }

    fn url(&self) -> String {
        let path = if self.path.starts_with('/') {
            self.path.clone()
        } else {
            format!("/{}", self.path)
        };

        format!("{}{}", self.base_url.trim_end_matches('/'), path)
    }

    fn headers(&self) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        if let Some(api_key) = &self.api_key {
            headers.insert("x-api-key", HeaderValue::from_str(api_key)?);
        }
        Ok(headers)
    }

    async fn fetch_once(&self, as_of_date: NaiveDate) -> Result<(DailyFeaturesResponse, Value)> {
        let url = self.url();
        let headers = self.headers()?;

        let res = self
            .http
            .get(url)
            .headers(headers)
            .query(&[("as_of_date", as_of_date.to_string())])
            .send()
            .await
            .context("data provider request failed")?;

        let status = res.status();
        let text = res
            .text()
            .await
            .context("failed to read provider response")?;
        let raw_json = serde_json::from_str::<Value>(&text)
            .with_context(|| format!("provider response is not valid JSON: {text}"))?;

        if !status.is_success() {
            anyhow::bail!("data provider HTTP {status}: {raw_json}");
        }

        let parsed = serde_json::from_value::<DailyFeaturesResponse>(raw_json.clone())
            .context("failed to parse provider response into DailyFeaturesResponse")?;
        Ok((parsed, raw_json))
    }

    fn validate(&self, resp: &DailyFeaturesResponse, expected: NaiveDate) -> Result<()> {
        anyhow::ensure!(
            resp.as_of_date == expected,
            "provider as_of_date mismatch: expected {expected}, got {}",
            resp.as_of_date
        );

        for item in &resp.items {
            validate_item(item)?;
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl DataProviderClient for HttpJsonDataProvider {
    fn provider_name(&self) -> &'static str {
        "external_http_json"
    }

    async fn fetch_daily_features(
        &self,
        as_of_date: NaiveDate,
    ) -> Result<(DailyFeaturesResponse, Value)> {
        let mut attempt: u32 = 0;
        loop {
            attempt += 1;
            let res = self.fetch_once(as_of_date).await;
            match res {
                Ok((parsed, raw)) => {
                    self.validate(&parsed, as_of_date)?;
                    return Ok((parsed, raw));
                }
                Err(err) => {
                    if attempt >= self.retries {
                        return Err(err);
                    }
                    let backoff = Duration::from_secs(1 << (attempt - 1));
                    tracing::warn!(attempt, ?backoff, error = %err, "data provider fetch failed; retrying");
                    tokio::time::sleep(backoff).await;
                }
            }
        }
    }
}

fn validate_item(item: &DailyFeatureItem) -> Result<()> {
    anyhow::ensure!(!item.ticker.trim().is_empty(), "ticker must be non-empty");
    anyhow::ensure!(!item.name.trim().is_empty(), "name must be non-empty");
    anyhow::ensure!(!item.features.is_empty(), "features must be non-empty");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::types::DailyFeaturesResponse;
    use serde_json::json;

    #[test]
    fn parses_expected_shape_and_numeric_features() {
        let as_of = NaiveDate::from_ymd_opt(2026, 1, 27).unwrap();
        let v = json!({
            "as_of_date": as_of,
            "items": [
                {
                    "ticker": "KRX:005930",
                    "name": "Samsung",
                    "trading_value": 123.0,
                    "features": {"ret_1d": 0.01, "mom_5d": -0.02}
                }
            ]
        });

        let parsed: DailyFeaturesResponse = serde_json::from_value(v).unwrap();
        assert_eq!(parsed.as_of_date, as_of);
        assert_eq!(parsed.items.len(), 1);
        assert_eq!(parsed.items[0].features.get("ret_1d").copied(), Some(0.01));
    }

    #[test]
    fn rejects_non_numeric_features_via_deserialize() {
        let as_of = NaiveDate::from_ymd_opt(2026, 1, 27).unwrap();
        let v = json!({
            "as_of_date": as_of,
            "items": [
                {
                    "ticker": "KRX:005930",
                    "name": "Samsung",
                    "trading_value": 123.0,
                    "features": {"ret_1d": "0.01"}
                }
            ]
        });

        let res = serde_json::from_value::<DailyFeaturesResponse>(v);
        assert!(res.is_err());
    }
}
