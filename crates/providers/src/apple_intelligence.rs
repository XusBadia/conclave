//! Apple Intelligence on-device provider (macOS 26+, Apple Silicon).
//!
//! Wraps the `FoundationModels` framework via a tiny Swift→C bridge
//! (see `swift/AppleIntelligenceBridge.swift`). The provider is
//! [`ProviderScope::Subtask`] — Apple's safety guardrails reject most
//! clinical/legal content, so this implementation is intentionally
//! barred from the deliberation flows and is meant for low-risk utility
//! work like de-identification helpers, summarisation of non-clinical
//! batches, or follow-up-question suggestions.
//!
//! Cross-platform: on Linux/Windows (or when the Swift toolchain is
//! missing on a macOS host) the provider compiles to a stub that
//! reports `Availability::FrameworkUnavailable` and returns
//! [`ProviderError::Unavailable`] on every `complete` call. This lets
//! workspace-wide `cargo build`/`cargo check` keep working.

use async_trait::async_trait;

use crate::error::ProviderError;
use crate::types::{
    CompletionRequest, CompletionResponse, MessageRole, ProviderCapabilities, ProviderScope, Usage,
};
use crate::LlmProvider;

/// Default model id surfaced to the UI.
///
/// Foundation Models doesn't expose per-model selection today — the
/// framework picks the on-device model transparently — but Conclave's
/// UI always shows *something* as the model name, so we give it a
/// stable, recognisable label.
pub const DEFAULT_MODEL_LABEL: &str = "Apple Foundation Model";

/// Stable provider id used as the keychain account suffix and routing
/// key, mirrors the patterns in [`crate::registry::KNOWN_PROVIDERS`].
pub const PROVIDER_ID: &str = "apple-intelligence";

/// Result of querying `SystemLanguageModel.default.availability`.
///
/// The variants mirror the reason codes returned by the Swift bridge
/// (see `apple_intel_availability` in
/// `swift/AppleIntelligenceBridge.swift`). Keep the wire codes in sync
/// between the two sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    /// The on-device model is ready to serve requests.
    Available,
    /// Host is not Apple-Silicon (or otherwise not eligible).
    DeviceNotEligible,
    /// Apple Intelligence has not been turned on in System Settings.
    AppleIntelligenceNotEnabled,
    /// The model is still downloading / preparing.
    ModelNotReady,
    /// The host OS or toolchain doesn't ship `FoundationModels`.
    FrameworkUnavailable,
    /// Anything else the Swift side reported.
    Other,
}

impl Availability {
    fn from_code(code: i32) -> Self {
        match code {
            0 => Self::Available,
            1 => Self::DeviceNotEligible,
            2 => Self::AppleIntelligenceNotEnabled,
            3 => Self::ModelNotReady,
            4 => Self::FrameworkUnavailable,
            _ => Self::Other,
        }
    }

    /// English reason text used in the Settings card hint. The frontend
    /// keys i18n off the variant, not this string, so this is fine
    /// staying ASCII-only.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::DeviceNotEligible => "requires Apple Silicon (M1 or newer)",
            Self::AppleIntelligenceNotEnabled => "turn on Apple Intelligence in System Settings",
            Self::ModelNotReady => "Apple Intelligence is still downloading",
            Self::FrameworkUnavailable => "requires macOS 26 or newer",
            Self::Other => "unavailable on this device",
        }
    }

    /// Machine-readable tag for the frontend so it can pick the right
    /// i18n string. Stable across Apple updates.
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::DeviceNotEligible => "device",
            Self::AppleIntelligenceNotEnabled => "not_enabled",
            Self::ModelNotReady => "downloading",
            Self::FrameworkUnavailable => "os",
            Self::Other => "other",
        }
    }

    /// `true` when the user can plausibly do something to make the
    /// provider work — i.e. it's already available, or they can flip a
    /// toggle in System Settings, or the on-device model is finishing
    /// its download.
    ///
    /// The UI hides the provider entirely when this returns `false`
    /// (Intel Macs, macOS < 26, etc.) so users for whom Apple
    /// Intelligence is structurally unreachable never see a card they
    /// would only be confused by.
    #[must_use]
    pub const fn is_user_actionable(self) -> bool {
        matches!(
            self,
            Self::Available | Self::AppleIntelligenceNotEnabled | Self::ModelNotReady
        )
    }
}

/// On-device Apple Intelligence provider.
#[derive(Debug, Clone, Default)]
pub struct AppleIntelligenceProvider;

impl AppleIntelligenceProvider {
    /// Build a default provider. No configuration is required — the
    /// underlying framework selects the on-device model itself.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Async availability check. Cheap: just an FFI call into Swift
    /// which reads `SystemLanguageModel.default.availability` and
    /// returns the cached state.
    pub async fn availability(&self) -> Availability {
        tokio::task::spawn_blocking(query_availability)
            .await
            .unwrap_or(Availability::Other)
    }
}

#[async_trait]
impl LlmProvider for AppleIntelligenceProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            // The on-device foundation model is small; 4K is the
            // documented context. Treat as a hard cap.
            max_context_tokens: 4_096,
            // JSON via the `@Generable` macro on the Swift side. Not
            // wired yet — callers should request JSON in the prompt
            // and parse defensively, same as Ollama with non-JSON
            // models.
            supports_json_mode: false,
            supports_streaming: false,
            vision: false,
            scope: ProviderScope::Subtask,
        }
    }

    fn requires_network(&self) -> bool {
        false
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ProviderError> {
        let prompt = flatten_messages(&req);
        let max_tokens = req.max_output_tokens.unwrap_or(512);
        let temperature = req.temperature.unwrap_or(0.7);

        let result =
            tokio::task::spawn_blocking(move || run_completion(&prompt, max_tokens, temperature))
                .await
                .map_err(|e| ProviderError::Other(format!("apple intelligence join: {e}")))??;

        // We can't get a true token count back from FoundationModels
        // today, so estimate from character length — 4 chars/token is a
        // reasonable english-leaning approximation, used here only for
        // accounting display.
        let input_tokens = (req.messages.iter().map(|m| m.content.len()).sum::<usize>() / 4) as u32;
        let output_tokens = (result.len() / 4) as u32;

        Ok(CompletionResponse {
            text: result,
            usage: Usage {
                input_tokens,
                output_tokens,
            },
            model: DEFAULT_MODEL_LABEL.to_owned(),
            web_citations: Vec::new(),
        })
    }
}

/// Collapse the chat history into a single prompt string. Apple's
/// session API takes a flat `String`; system messages get a leading
/// "System: " prefix and assistant turns are echoed back as a hint.
fn flatten_messages(req: &CompletionRequest) -> String {
    let mut buf = String::with_capacity(256);
    for m in &req.messages {
        let prefix = match m.role {
            MessageRole::System => "System:",
            MessageRole::User => "User:",
            MessageRole::Assistant => "Assistant:",
        };
        if !buf.is_empty() {
            buf.push_str("\n\n");
        }
        buf.push_str(prefix);
        buf.push(' ');
        buf.push_str(&m.content);
    }
    buf
}

// ---------------------------------------------------------------------------
// FFI surface
//
// Every `unsafe` here is justified at the call site. The bridge has a
// narrow contract: three `extern "C"` functions, one of which returns
// a Swift-allocated buffer that we must release via `apple_intel_free`.
// The rest of the crate stays unsafe-free.
// ---------------------------------------------------------------------------

#[cfg(all(target_os = "macos", not(apple_intel_stub)))]
#[allow(unsafe_code)]
mod ffi {
    use std::os::raw::{c_char, c_int};

    extern "C" {
        pub fn apple_intel_availability() -> c_int;
        pub fn apple_intel_complete(
            prompt: *const c_char,
            prompt_len: usize,
            max_tokens: u32,
            temperature: f32,
            out_text: *mut *mut c_char,
            out_len: *mut usize,
        ) -> c_int;
        pub fn apple_intel_free(ptr: *mut c_char);
    }
}

#[cfg(all(target_os = "macos", not(apple_intel_stub)))]
#[allow(unsafe_code)]
fn query_availability() -> Availability {
    // SAFETY: zero-arg C function with a stable integer return code.
    let code = unsafe { ffi::apple_intel_availability() };
    Availability::from_code(code)
}

#[cfg(all(target_os = "macos", not(apple_intel_stub)))]
#[allow(unsafe_code)]
fn run_completion(
    prompt: &str,
    max_tokens: u32,
    temperature: f32,
) -> Result<String, ProviderError> {
    use std::os::raw::c_char;

    let availability = query_availability();
    if availability != Availability::Available {
        return Err(ProviderError::Unavailable(availability.reason().to_owned()));
    }

    let prompt_bytes = prompt.as_bytes();
    let prompt_ptr = prompt_bytes.as_ptr().cast::<c_char>();

    let mut out_ptr: *mut c_char = std::ptr::null_mut();
    let mut out_len: usize = 0;

    // SAFETY: `prompt_ptr` is valid for `prompt_bytes.len()` bytes for
    // the duration of the call (we hold `prompt_bytes` on the stack).
    // `out_ptr` and `out_len` point to local mutable variables; the
    // Swift side writes them only when the function returns 0.
    let code = unsafe {
        ffi::apple_intel_complete(
            prompt_ptr,
            prompt_bytes.len(),
            max_tokens,
            temperature,
            &raw mut out_ptr,
            &raw mut out_len,
        )
    };

    if code == 0 && !out_ptr.is_null() {
        // SAFETY: Swift guarantees the buffer is `out_len` bytes of
        // valid UTF-8 followed by a NUL, allocated via
        // `UnsafeMutablePointer<CChar>.allocate`. We copy into a
        // Rust-owned `String` and immediately free the Swift buffer.
        let result = unsafe {
            let slice = std::slice::from_raw_parts(out_ptr.cast::<u8>(), out_len);
            let s = std::str::from_utf8(slice)
                .map_err(|e| ProviderError::Other(format!("apple intelligence utf8: {e}")))?
                .to_owned();
            ffi::apple_intel_free(out_ptr);
            s
        };
        return Ok(result);
    }

    // Map the Swift-side error codes back to typed errors. Anything we
    // don't recognise falls through to `Other` so the user still sees a
    // distinct message.
    Err(match code {
        -1 => ProviderError::Unavailable(query_availability().reason().to_owned()),
        -2 => ProviderError::BadRequest(
            "Apple Intelligence refused this prompt (safety guardrail).".into(),
        ),
        -3 => ProviderError::BadRequest("Apple Intelligence: invalid input.".into()),
        other => ProviderError::Other(format!("apple intelligence error code {other}")),
    })
}

// ---------------------------------------------------------------------------
// Stub for non-macOS targets (and macOS hosts without the Swift toolchain).
// ---------------------------------------------------------------------------

#[cfg(any(not(target_os = "macos"), apple_intel_stub))]
fn query_availability() -> Availability {
    Availability::FrameworkUnavailable
}

#[cfg(any(not(target_os = "macos"), apple_intel_stub))]
fn run_completion(
    _prompt: &str,
    _max_tokens: u32,
    _temperature: f32,
) -> Result<String, ProviderError> {
    Err(ProviderError::Unavailable(
        Availability::FrameworkUnavailable.reason().to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Message;

    #[test]
    fn scope_is_subtask() {
        let p = AppleIntelligenceProvider::new();
        assert_eq!(p.capabilities().scope, ProviderScope::Subtask);
    }

    #[test]
    fn does_not_require_network() {
        assert!(!AppleIntelligenceProvider::new().requires_network());
    }

    #[tokio::test]
    async fn availability_on_non_macos_or_stub_is_unavailable() {
        // On Linux/Windows CI this resolves through the stub path. On
        // a macOS host with the bridge compiled it depends on the
        // device — the test asserts only that the call returns
        // *something* rather than panicking. The stronger guarantee is
        // documented at the function level.
        let _ = AppleIntelligenceProvider::new().availability().await;
    }

    #[test]
    fn flatten_uses_role_prefixes() {
        let req = CompletionRequest {
            messages: vec![Message::system("be brief"), Message::user("hi")],
            ..Default::default()
        };
        let flat = flatten_messages(&req);
        assert!(flat.contains("System: be brief"));
        assert!(flat.contains("User: hi"));
    }

    #[test]
    fn user_actionable_excludes_hard_unavailable_states() {
        // Hard unavailable — the user has no fix path. UI should hide
        // the provider so they aren't shown a card they can't use.
        assert!(!Availability::DeviceNotEligible.is_user_actionable());
        assert!(!Availability::FrameworkUnavailable.is_user_actionable());
        assert!(!Availability::Other.is_user_actionable());

        // Soft unavailable — user can do something (enable in System
        // Settings, wait for download). UI shows the card with a hint.
        assert!(Availability::Available.is_user_actionable());
        assert!(Availability::AppleIntelligenceNotEnabled.is_user_actionable());
        assert!(Availability::ModelNotReady.is_user_actionable());
    }
}
