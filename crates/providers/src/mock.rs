//! In-memory provider that returns canned responses. Used by tests and the
//! verdict-engine unit tests so CI never hits the network.

use std::sync::Mutex;

use async_trait::async_trait;

use crate::error::ProviderError;
use crate::types::{
    CompletionRequest, CompletionResponse, ProviderCapabilities, ProviderScope, Usage,
};
use crate::LlmProvider;

/// Deterministic mock provider.
#[derive(Debug)]
pub struct MockProvider {
    id: &'static str,
    responses: Mutex<Vec<String>>,
    cursor: Mutex<usize>,
    captured: Mutex<Vec<CompletionRequest>>,
}

impl MockProvider {
    /// Build a provider that cycles through `responses`. When the list is
    /// exhausted, the last entry is repeated.
    #[must_use]
    pub const fn new(responses: Vec<String>) -> Self {
        Self {
            id: "mock",
            responses: Mutex::new(responses),
            cursor: Mutex::new(0),
            captured: Mutex::new(Vec::new()),
        }
    }

    /// Convenience: a single canned response.
    pub fn with_response(response: impl Into<String>) -> Self {
        Self::new(vec![response.into()])
    }

    /// Inspect everything passed to [`complete`] so far.
    pub fn captured_requests(&self) -> Vec<CompletionRequest> {
        self.captured.lock().map(|v| v.clone()).unwrap_or_default()
    }
}

#[async_trait]
impl LlmProvider for MockProvider {
    fn id(&self) -> &'static str {
        self.id
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            max_context_tokens: 200_000,
            supports_json_mode: true,
            supports_streaming: false,
            vision: false,
            scope: ProviderScope::General,
        }
    }

    fn requires_network(&self) -> bool {
        false
    }

    // Two mutex locks held in sequence inside the same scope so the cursor
    // advances atomically against the responses snapshot — clippy's
    // tightening heuristic doesn't see that intent.
    #[allow(clippy::significant_drop_tightening)]
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ProviderError> {
        if let Ok(mut buf) = self.captured.lock() {
            buf.push(req);
        }

        let responses = self
            .responses
            .lock()
            .map_err(|_| ProviderError::Other("mock lock poisoned".into()))?;
        if responses.is_empty() {
            return Err(ProviderError::Other(
                "mock provider has no canned responses".into(),
            ));
        }
        let mut cursor = self
            .cursor
            .lock()
            .map_err(|_| ProviderError::Other("mock cursor poisoned".into()))?;
        let idx = (*cursor).min(responses.len() - 1);
        *cursor += 1;
        let text = responses[idx].clone();
        Ok(CompletionResponse {
            text,
            usage: Usage {
                input_tokens: 1,
                output_tokens: 1,
            },
            model: "mock-model".into(),
            web_citations: Vec::new(),
        })
    }
}
