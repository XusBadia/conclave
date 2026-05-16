//! Typed error type for the provider layer.

/// Failure modes shared by every provider implementation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ProviderError {
    /// Transport-level failure (DNS, TCP, TLS, body read).
    #[error("network error: {0}")]
    Network(String),
    /// 401/403 from the provider — credentials missing or rejected.
    #[error("authentication failed")]
    Auth,
    /// 429 with optional `Retry-After` hint in seconds.
    #[error("rate limited{}", retry_after_secs.map_or_else(String::new, |s| format!(" (retry after {s}s)")))]
    RateLimit {
        /// Hint from the provider's `Retry-After` header, if any.
        retry_after_secs: Option<u32>,
    },
    /// 4xx that isn't auth/rate-limit — usually a malformed request.
    #[error("bad request: {0}")]
    BadRequest(String),
    /// Request would exceed the model's context window.
    #[error("context window overflow")]
    ContextOverflow,
    /// Anything else: parse failure, unexpected status, etc.
    #[error("provider error: {0}")]
    Other(String),
}
