use crate::config::Settings;
use crate::domain::recommendation::RecommendationSnapshot;
use crate::llm::json;
use crate::llm::{GenerateInput, LlmClient, Provider};
use crate::llm::error::LlmDiagnosticsError;
use anyhow::{anyhow, Context};
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use std::time::Duration;

const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const DEFAULT_MODEL: &str = "claude-3-5-sonnet-latest";
const DEFAULT_MAX_TOKENS: u32 = 2048;
const DEFAULT_TIMEOUT_SECS: u64 = 60;

#[derive(Debug, Clone)]
pub struct AnthropicClient {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
    model: String,
    max_tokens: u32,
}

impl AnthropicClient {
    pub fn from_settings(settings: &Settings) -> anyhow::Result<Self> {
        let api_key = settings.require_anthropic_api_key()?.to_string();
        let base_url = std::env::var("ANTHROPIC_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        let model = std::env::var("ANTHROPIC_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
        let max_tokens = std::env::var("ANTHROPIC_MAX_TOKENS")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(DEFAULT_MAX_TOKENS);

        let timeout_secs = std::env::var("ANTHROPIC_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_TIMEOUT_SECS);

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .context("failed to build reqwest client")?;

        Ok(Self {
            http,
            api_key,
            base_url,
            model,
            max_tokens,
        })
    }

    async fn create_message(&self, req: CreateMessageRequest) -> anyhow::Result<(serde_json::Value, CreateMessageResponse)> {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_str(&self.api_key)?);
        headers.insert("anthropic-version", HeaderValue::from_static(ANTHROPIC_VERSION));

        let url = format!("{}/v1/messages", self.base_url.trim_end_matches('/'));
        let res = self
            .http
            .post(url)
            .headers(headers)
            .json(&req)
            .send()
            .await
            .context("Anthropic request failed")?;

        let status = res.status();
        let text = res.text().await.context("failed to read Anthropic response body")?;
        if !status.is_success() {
            let raw_response_json = serde_json::from_str::<serde_json::Value>(&text).ok();
            return Err(LlmDiagnosticsError {
                provider: Provider::Anthropic,
                stage: "http",
                detail: format!("status={status}"),
                raw_output: Some(text),
                raw_response_json,
            }
            .into());
        }

        let raw_json = serde_json::from_str::<serde_json::Value>(&text)
            .with_context(|| format!("failed to parse Anthropic response JSON: {text}"))?;
        let parsed = serde_json::from_value::<CreateMessageResponse>(raw_json.clone())
            .context("failed to decode Anthropic response into CreateMessageResponse")?;
        Ok((raw_json, parsed))
    }

    fn system_prompt() -> String {
        // Keep strict and provider-agnostic: JSON only, no prose.
        [
            "You are a stock recommendation engine for KRX.",
            "Return ONLY valid JSON. Do not wrap in markdown. Do not include any extra keys.",
            "Output schema:",
            "{",
            "  \"as_of_date\": \"YYYY-MM-DD\",",
            "  \"generated_at\": \"ISO-8601\",",
            "  \"items\": [",
            "    {",
            "      \"rank\": 1,",
            "      \"ticker\": \"KRX:005930\",",
            "      \"name\": \"삼성전자\",",
            "      \"rationale\": [\"line1\", \"line2\", \"line3\"],",
            "      \"risk_notes\": \"optional\",",
            "      \"confidence\": 0.0",
            "    }",
            "  ]",
            "}",
            "Rules:",
            "- items must have exactly 20 entries, ranks 1..20 unique",
            "- rationale must have exactly 3 short lines per item",
            "- confidence (if present) must be in [0, 1]",
            "- Use only the provided candidates (ticker/name)",
        ]
        .join("\n")
    }

    fn user_prompt(input: &GenerateInput) -> String {
        format!(
            "Task: Select the top 20 short-term (<= 1 week) recommendations for as_of_date={}.\n\nCandidates JSON:\n{}",
            input.as_of_date,
            input.candidates_json()
        )
    }

    fn repair_prompt(previous_output: &str, expected_as_of_date: chrono::NaiveDate) -> String {
        format!(
            "The previous output was invalid. Re-emit ONLY valid JSON that matches the schema and rules. \
The JSON must have as_of_date={expected_as_of_date}.\n\nInvalid output:\n{previous_output}"
        )
    }

    fn parse_snapshot(text: &str, expected_as_of_date: chrono::NaiveDate) -> anyhow::Result<RecommendationSnapshot> {
        json::parse_snapshot(text, expected_as_of_date)
    }

    fn response_text(res: &CreateMessageResponse) -> anyhow::Result<String> {
        let mut out = String::new();
        for block in &res.content {
            match block {
                ContentBlock::Text { text } => {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(text);
                }
                ContentBlock::ToolUse { .. } => {
                    return Err(anyhow!("Anthropic returned tool_use content; tool execution is not supported"));
                }
                ContentBlock::Thinking { .. } | ContentBlock::RedactedThinking { .. } => {
                    // Ignore.
                }
                ContentBlock::Unknown => {
                    // Ignore unknown blocks.
                }
            }
        }
        Ok(out)
    }
}

#[async_trait::async_trait]
impl LlmClient for AnthropicClient {
    fn provider(&self) -> Provider {
        Provider::Anthropic
    }

    async fn generate_recommendations(&self, input: GenerateInput) -> anyhow::Result<RecommendationSnapshot> {
        let (snapshot, _raw) = self.generate_recommendations_with_raw(input).await?;
        Ok(snapshot)
    }
}

impl AnthropicClient {
    pub async fn generate_recommendations_with_raw(
        &self,
        input: GenerateInput,
    ) -> anyhow::Result<(RecommendationSnapshot, serde_json::Value)> {
        let req = CreateMessageRequest {
            model: self.model.clone(),
            max_tokens: self.max_tokens,
            system: Some(Self::system_prompt()),
            messages: vec![Message {
                role: "user",
                content: Self::user_prompt(&input),
            }],
        };

        let (raw_json, res) = self.create_message(req).await?;
        let text = Self::response_text(&res)?;
        match Self::parse_snapshot(&text, input.as_of_date) {
            Ok(snapshot) => Ok((snapshot, raw_json)),
            Err(first_err) => {
                let repair_req = CreateMessageRequest {
                    model: self.model.clone(),
                    max_tokens: self.max_tokens,
                    system: Some(Self::system_prompt()),
                    messages: vec![Message {
                        role: "user",
                        content: Self::repair_prompt(&text, input.as_of_date),
                    }],
                };

                let (repair_raw_json, repair_res) = self.create_message(repair_req).await?;
                let repair_text = Self::response_text(&repair_res)?;
                match Self::parse_snapshot(&repair_text, input.as_of_date) {
                    Ok(snapshot) => Ok((snapshot, repair_raw_json)),
                    Err(second_err) => Err(LlmDiagnosticsError {
                        provider: Provider::Anthropic,
                        stage: "parse_after_repair",
                        detail: format!("first_error={first_err}; second_error={second_err}"),
                        raw_output: Some(repair_text),
                        raw_response_json: Some(repair_raw_json),
                    }
                    .into()),
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct CreateMessageRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<Message>,
}

#[derive(Debug, Clone, Serialize)]
struct Message {
    role: &'static str,
    content: String,
}

#[derive(Debug, Clone, Deserialize)]
struct CreateMessageResponse {
    content: Vec<ContentBlock>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },

    #[serde(rename = "tool_use")]
    ToolUse {
        #[serde(default)]
        id: String,
        #[serde(default)]
        name: String,
        #[serde(default)]
        input: serde_json::Value,
    },

    #[serde(rename = "thinking")]
    Thinking {
        #[serde(default)]
        thinking: String,
        #[serde(default)]
        signature: String,
    },

    #[serde(rename = "redacted_thinking")]
    RedactedThinking {
        #[serde(default)]
        data: String,
    },

    #[serde(other)]
    Unknown,
}
