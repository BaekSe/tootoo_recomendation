use crate::llm::Provider;
use std::fmt;

#[derive(Debug, Clone)]
pub struct LlmDiagnosticsError {
    pub provider: Provider,
    pub stage: &'static str,
    pub detail: String,
    pub raw_output: Option<String>,
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
