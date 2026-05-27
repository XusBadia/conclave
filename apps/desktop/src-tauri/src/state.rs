//! Shared runtime state held by the Tauri app.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use conclave_core::{paths::Paths, Config};
use conclave_providers::AnthropicLoginFlow;
use conclave_rag::{DocumentRepository, Embedder, FastEmbedEmbedder};
use tokio::task::AbortHandle;

use crate::commands::ProviderStatus;

/// One per app instance.
pub struct AppState {
    pub paths: Paths,
    pub config: Mutex<Config>,
    /// In-flight Anthropic OAuth login (PKCE pair + state) waiting for the
    /// user to paste their code. At most one flow may be active at a time.
    pub anthropic_login: Mutex<Option<AnthropicLoginFlow>>,
    /// In-flight OpenAI OAuth task. Holding the abort handle here lets the
    /// UI cancel a stuck flow, which drops the localhost:1455 listener so
    /// the next attempt can bind cleanly.
    pub openai_login: Mutex<Option<AbortHandle>>,
    /// Embedder shared across every RAG command. `FastEmbedEmbedder::new()` is
    /// cheap (it does not download the model until `embed()` is first called),
    /// so building it at startup keeps the cold path bounded to the first
    /// ingestion call rather than every single command.
    pub embedder: Arc<dyn Embedder>,
    /// Per-workspace cached repository handles. Opening a `DocumentRepository`
    /// touches both SQLite and LanceDB, so we hold them across calls.
    pub repos: tokio::sync::Mutex<HashMap<String, Arc<DocumentRepository>>>,
    /// Flipped to `true` by the `ingest_cancel` command to ask an in-flight
    /// ingestion batch to stop after the file currently being processed.
    pub ingest_cancel: Arc<AtomicBool>,
    /// Flipped to `true` by the `batch_cancel` command to stop a batch
    /// case run after the case currently being processed. Cleared
    /// whenever a new batch begins.
    pub batch_cancel: Arc<AtomicBool>,
    /// Per-case cancellation flags. The batch worker registers an
    /// `AtomicBool` for every case it runs; `cancel_case(case_id)` flips
    /// it so the deliberation (or quick) pipeline short-circuits at the
    /// next phase boundary. Cleared after the case worker resolves.
    pub case_cancels: tokio::sync::Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Probe-result cache for `list_providers`. Maps provider id to the
    /// last probe outcome plus the wall-clock instant it was recorded.
    /// Entries are reused while still within `PROBE_TTL` (60 s) so
    /// repeated `list_providers` calls from the UI don't hammer the
    /// upstream provider once per render. The `force_refresh: true`
    /// path on `list_providers` bypasses this cache. Wrapped in `Arc`
    /// so a background task (e.g. the OpenAI OAuth callback waiter)
    /// can keep a handle and invalidate entries after the new
    /// credentials land on disk.
    pub probe_cache: Arc<tokio::sync::Mutex<HashMap<String, (Instant, ProviderStatus)>>>,
}

impl AppState {
    pub fn new(paths: Paths, config: Config) -> Self {
        // Pin fastembed's model cache under the OS app cache dir so launches
        // from different CWDs (Tauri dev, `open .app`, packaged release)
        // all hit the same on-disk ONNX model and never re-download.
        let fastembed_cache = paths.cache_dir().join("fastembed");
        let embedder: Arc<dyn Embedder> =
            Arc::new(FastEmbedEmbedder::new().with_cache_dir(fastembed_cache));
        Self {
            paths,
            config: Mutex::new(config),
            anthropic_login: Mutex::new(None),
            openai_login: Mutex::new(None),
            embedder,
            repos: tokio::sync::Mutex::new(HashMap::new()),
            ingest_cancel: Arc::new(AtomicBool::new(false)),
            batch_cancel: Arc::new(AtomicBool::new(false)),
            case_cancels: tokio::sync::Mutex::new(HashMap::new()),
            probe_cache: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }
}
