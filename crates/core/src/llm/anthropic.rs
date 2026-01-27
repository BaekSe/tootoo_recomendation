// Anthropic provider implementation (WIP).
//
// Docs:
// - POST https://api.anthropic.com/v1/messages
// - Headers: x-api-key, anthropic-version: 2023-06-01, content-type: application/json

#[derive(Debug, Clone)]
pub struct AnthropicClient {
    _private: (),
}

impl AnthropicClient {
    pub fn new() -> Self {
        Self { _private: () }
    }
}
