use crate::llm::Provider;
use serde_json::Value;
use std::fmt;

#[derive(Debug, Clone)]
pub struct LlmDiagnosticsError {
    pub provider: Provider,
    pub stage: &'static str,
    pub detail: String,
    pub raw_output: Option<String>,
    pub raw_response_json: Option<Value>,
}

impl fmt::Display for LlmDiagnosticsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "LLM error (provider={:?}, stage={}): {}",
            self.provider, self.stage, self.detail
        )
    }
}

impl std::error::Error for LlmDiagnosticsError {}
