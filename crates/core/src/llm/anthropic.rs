use crate::config::Settings;
use crate::domain::contract::LlmRecommendationSnapshot;
use crate::domain::recommendation::RecommendationSnapshot;
use crate::llm::error::LlmDiagnosticsError;
use crate::llm::json;
use crate::llm::{GenerateInput, LlmClient, Provider};
use anyhow::Context;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use std::time::Duration;

const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const DEFAULT_MODEL: &str = "claude-3-5-sonnet-latest";
const DEFAULT_MAX_TOKENS: u32 = 2048;
const DEFAULT_TIMEOUT_SECS: u64 = 60;

const TOOL_NAME_EMIT_SNAPSHOT: &str = "emit_snapshot";

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
        let base_url =
            std::env::var("ANTHROPIC_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
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

    async fn create_message(
        &self,
        req: CreateMessageRequest,
    ) -> anyhow::Result<(serde_json::Value, CreateMessageResponse)> {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_str(&self.api_key)?);
        headers.insert(
            "anthropic-version",
            HeaderValue::from_static(ANTHROPIC_VERSION),
        );

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
        let text = res
            .text()
            .await
            .context("failed to read Anthropic response body")?;
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

    fn tools() -> Vec<Tool> {
        // Minimal JSON schema for the exact snapshot contract.
        // Keep it strict and explicit to maximize compliance.
        let schema = serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["as_of_date", "generated_at", "items"],
            "properties": {
                "as_of_date": {"type": "string"},
                "generated_at": {"type": "string"},
                "items": {
                    "type": "array",
                    "minItems": 20,
                    "maxItems": 20,
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["rank", "ticker", "name", "rationale", "risk_notes", "confidence"],
                        "properties": {
                            "rank": {"type": "integer"},
                            "ticker": {"type": "string"},
                            "name": {"type": "string"},
                            "rationale": {
                                "type": "array",
                                "minItems": 3,
                                "maxItems": 3,
                                "items": {"type": "string"}
                            },
                            "risk_notes": {"type": ["string", "null"]},
                            "confidence": {"type": ["number", "null"]}
                        }
                    }
                }
            }
        });

        vec![Tool {
            name: TOOL_NAME_EMIT_SNAPSHOT,
            description: "Emit the final recommendation snapshot as structured JSON",
            input_schema: schema,
        }]
    }

    fn tool_choice() -> ToolChoice {
        ToolChoice::Tool {
            name: TOOL_NAME_EMIT_SNAPSHOT,
        }
    }

    fn system_prompt() -> String {
        // Keep strict and provider-agnostic: JSON only, no prose.
        [
            "You are a stock recommendation engine for KRX.",
            "Return ONLY valid JSON. Do not wrap in markdown. Do not include any extra keys.",
            "No trailing commas. No comments. No semicolons. Use double quotes for all JSON strings.",
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
            "- risk_notes key MUST be present (use null if none)",
            "- confidence key MUST be present (use null if unknown)",
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
        let schema = [
            "{",
            "  \"as_of_date\": \"YYYY-MM-DD\",",
            "  \"generated_at\": \"ISO-8601\",",
            "  \"items\": [",
            "    {",
            "      \"rank\": 1,",
            "      \"ticker\": \"KRX:005930\",",
            "      \"name\": \"삼성전자\",",
            "      \"rationale\": [\"line1\", \"line2\", \"line3\"],",
            "      \"risk_notes\": null,",
            "      \"confidence\": null",
            "    }",
            "  ]",
            "}",
        ]
        .join("\n");

        format!(
            "Your previous message was NOT valid JSON.\n\n\
TASK: Output ONLY a single JSON object that exactly matches the schema and rules.\n\
- Do NOT include any markdown, prose, or code fences.\n\
- Do NOT include trailing commas, comments, or semicolons.\n\
- Use double quotes for all JSON strings.\n\
- The JSON MUST have as_of_date=\"{expected_as_of_date}\".\n\
- The JSON MUST have exactly 20 items with ranks 1..20.\n\
- Each item MUST include keys: rank, ticker, name, rationale, risk_notes, confidence.\n\
- rationale MUST have exactly 3 strings.\n\n\
SCHEMA:\n{schema}\n\n\
INVALID OUTPUT (for reference only; DO NOT copy verbatim):\n{previous_output}"
        )
    }

    fn parse_snapshot(
        text: &str,
        expected_as_of_date: chrono::NaiveDate,
    ) -> anyhow::Result<RecommendationSnapshot> {
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
                    // Prefer tool output parsing when tools are enabled.
                    // Callers should use `response_tool_snapshot`.
                    continue;
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

    fn response_tool_snapshot(res: &CreateMessageResponse) -> anyhow::Result<Option<LlmRecommendationSnapshot>> {
        for block in &res.content {
            if let ContentBlock::ToolUse { name, input, .. } = block {
                if name == TOOL_NAME_EMIT_SNAPSHOT {
                    let parsed = serde_json::from_value::<LlmRecommendationSnapshot>(input.clone())
                        .context("failed to decode tool_use.input into LlmRecommendationSnapshot")?;
                    return Ok(Some(parsed));
                }
            }
        }
        Ok(None)
    }
}

#[async_trait::async_trait]
impl LlmClient for AnthropicClient {
    fn provider(&self) -> Provider {
        Provider::Anthropic
    }

    async fn generate_recommendations(
        &self,
        input: GenerateInput,
    ) -> anyhow::Result<RecommendationSnapshot> {
        let (snapshot, _raw) = self.generate_recommendations_with_raw(input).await?;
        Ok(snapshot)
    }
}

impl AnthropicClient {
    async fn try_parse_with_repairs(
        &self,
        input: &GenerateInput,
        initial_text: String,
        initial_raw_json: serde_json::Value,
    ) -> anyhow::Result<(RecommendationSnapshot, serde_json::Value)> {
        match Self::parse_snapshot(&initial_text, input.as_of_date) {
            Ok(snapshot) => return Ok((snapshot, initial_raw_json)),
            Err(first_err) => {
                let mut last_err = first_err;
                let mut last_text = initial_text;
                let mut last_raw_json = initial_raw_json;

                // Repair attempts: 2
                for attempt in 1..=2u32 {
                    let repair_req = CreateMessageRequest {
                        model: self.model.clone(),
                        max_tokens: self.max_tokens,
                        system: Some(Self::system_prompt()),
                        messages: vec![Message {
                            role: "user",
                            content: Self::repair_prompt(&last_text, input.as_of_date),
                        }],
                        tools: Some(Self::tools()),
                        tool_choice: Some(Self::tool_choice()),
                    };

                    let (repair_raw_json, repair_res) = self.create_message(repair_req).await?;
                    let repair_text = Self::response_text(&repair_res)?;
                    match Self::parse_snapshot(&repair_text, input.as_of_date) {
                        Ok(snapshot) => return Ok((snapshot, repair_raw_json)),
                        Err(err) => {
                            last_err = err;
                            last_text = repair_text;
                            last_raw_json = repair_raw_json;
                            tracing::warn!(
                                attempt,
                                %input.as_of_date,
                                error = %last_err,
                                "LLM output still invalid after repair attempt"
                            );
                        }
                    }
                }

                Err(LlmDiagnosticsError {
                    provider: Provider::Anthropic,
                    stage: "parse_after_repair",
                    detail: format!("final_error={last_err}"),
                    raw_output: Some(last_text),
                    raw_response_json: Some(last_raw_json),
                }
                .into())
            }
        }
    }

    pub async fn generate_recommendations_with_raw(
        &self,
        input: GenerateInput,
    ) -> anyhow::Result<(RecommendationSnapshot, serde_json::Value)> {
        let make_req = |max_tokens: u32| CreateMessageRequest {
            model: self.model.clone(),
            max_tokens,
            system: Some(Self::system_prompt()),
            messages: vec![Message {
                role: "user",
                content: Self::user_prompt(&input),
            }],
            tools: Some(Self::tools()),
            tool_choice: Some(Self::tool_choice()),
        };

        let (mut raw_json, mut res) = self.create_message(make_req(self.max_tokens)).await?;

        // If the model hit max_tokens, retry once with a higher ceiling.
        if matches!(res.stop_reason.as_deref(), Some("max_tokens")) {
            let bumped = self.max_tokens.saturating_mul(2).max(4096);
            tracing::warn!(
                %input.as_of_date,
                from = self.max_tokens,
                to = bumped,
                "Anthropic stop_reason=max_tokens; retrying once with higher max_tokens"
            );
            let (rj, r) = self.create_message(make_req(bumped)).await?;
            raw_json = rj;
            res = r;
        }

        // Tool output path.
        if let Some(tool_snapshot) = Self::response_tool_snapshot(&res)? {
            let snapshot = tool_snapshot.validate_and_into_snapshot(input.as_of_date)?;
            return Ok((snapshot, raw_json));
        }

        // Fallback to text (should be rare).
        let text = Self::response_text(&res)?;
        self.try_parse_with_repairs(&input, text, raw_json).await
    }
}

#[derive(Debug, Clone, Serialize)]
struct CreateMessageRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<Message>,

    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<Tool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<ToolChoice>,
}

#[derive(Debug, Clone, Serialize)]
struct Message {
    role: &'static str,
    content: String,
}

#[derive(Debug, Clone, Deserialize)]
struct CreateMessageResponse {
    content: Vec<ContentBlock>,

    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct Tool {
    name: &'static str,
    description: &'static str,
    input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
enum ToolChoice {
    #[serde(rename = "tool")]
    Tool { name: &'static str },
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, TimeZone, Utc};
    use serde_json::json;

    #[test]
    fn parses_tool_use_snapshot_input() {
        let as_of = NaiveDate::from_ymd_opt(2026, 1, 28).unwrap();
        let generated_at = Utc.with_ymd_and_hms(2026, 1, 28, 9, 0, 0).unwrap();
        let items: Vec<_> = (1..=20)
            .map(|rank| {
                json!({
                    "rank": rank,
                    "ticker": format!("KRX:{rank:06}"),
                    "name": format!("Name {rank}"),
                    "rationale": ["a", "b", "c"],
                    "risk_notes": null,
                    "confidence": 0.5,
                })
            })
            .collect();

        let tool_input = json!({
            "as_of_date": as_of,
            "generated_at": generated_at,
            "items": items,
        });

        let res = CreateMessageResponse {
            content: vec![ContentBlock::ToolUse {
                id: "toolu_1".to_string(),
                name: TOOL_NAME_EMIT_SNAPSHOT.to_string(),
                input: tool_input,
            }],
            stop_reason: None,
        };

        let parsed = AnthropicClient::response_tool_snapshot(&res).unwrap().unwrap();
        let snapshot = parsed.validate_and_into_snapshot(as_of).unwrap();
        assert_eq!(snapshot.as_of_date, as_of);
        assert_eq!(snapshot.items.len(), 20);
        assert_eq!(snapshot.items[0].rank, 1);
    }
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
