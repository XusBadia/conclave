//! Tauri commands — thin wrappers over the Rust core crates. Every error
//! is mapped to a String so the frontend can render it directly.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

use conclave_core::{Workspace, WorkspaceManager};
use conclave_deident::{Deidentifier, PipelineDeidentifier};
use conclave_evidence::{
    EuropePmcSource, EvidenceCache, EvidenceItem, EvidenceSource, PubMedSource,
};
use conclave_providers::{
    open_in_browser, persist_tokens, secrets, AnthropicLoginFlow, AnthropicOAuthProvider,
    AnthropicProvider, AppleIntelligenceAvailability, AppleIntelligenceProvider, ClaudeCliProvider,
    CodexCliProvider, CompletionRequest, ImageInput, LlmProvider, Message, OllamaProvider,
    OpenAILoginFlow, OpenAIOAuthProvider, OpenAiProvider, OpenRouterProvider, ProbeDetails,
    ProviderScope, APPLE_INTELLIGENCE_MODEL_LABEL, CLAUDE_CLI_DEFAULT_MODEL, CLI_PROVIDERS,
    CODEX_CLI_DEFAULT_MODEL, KNOWN_PROVIDERS, OAUTH_PROVIDERS,
};
use conclave_rag::{
    ChunkParams, DocumentRecord, DocumentRepository, IngestionEvent, IngestionPipeline,
    ProgressStage, RepositoryLayout, SkipReason,
};
use conclave_verdict::{
    deliberation::{
        run_deliberation, DeliberationEvent, DeliberationEvidence, DeliberationInputs,
        DeliberationOptions, DeliberationPastCase,
    },
    ingest_case_attachments,
    persistence::{
        DeliberationTrace, FeedbackKind, FeedbackRecord, RetrievalTrace as VerdictRetrievalTrace,
    },
    sha256_hex, AuditPayloadMode, CaseAttachment, CaseRecord, CaseStore, DataBoundaryMode,
    QaPipeline, RawTextRetention, ReviewDecision, ReviewMetadataRecord, Skill, Verdict,
    VerdictOptions, VerdictPipeline, VerdictRecord,
};

use crate::state::AppState;

type CommandResult<T> = Result<T, String>;

fn ok<T>(t: T) -> CommandResult<T> {
    Ok(t)
}

fn err<T>(msg: impl std::fmt::Display) -> CommandResult<T> {
    Err(msg.to_string())
}

// ---------------------------------------------------------------------------
// Onboarding
// ---------------------------------------------------------------------------

const DISCLAIMER_MARKER: &str = "disclaimer-accepted-v1";

#[derive(Debug, Serialize)]
pub struct OnboardingStatus {
    pub accepted: bool,
    /// English disclaimer copy — kept for backwards compatibility with the
    /// previous frontend contract.
    pub disclaimer: String,
    pub disclaimer_en: String,
    pub disclaimer_es: String,
}

#[tauri::command]
pub fn onboarding_status(state: State<'_, AppState>) -> CommandResult<OnboardingStatus> {
    let path = state.paths.config_dir().join(DISCLAIMER_MARKER);
    Ok(OnboardingStatus {
        accepted: path.exists(),
        disclaimer: conclave_core::MEDICAL_DISCLAIMER_EN.to_owned(),
        disclaimer_en: conclave_core::MEDICAL_DISCLAIMER_EN.to_owned(),
        disclaimer_es: conclave_core::MEDICAL_DISCLAIMER_ES.to_owned(),
    })
}

#[tauri::command]
pub fn accept_disclaimer(state: State<'_, AppState>) -> CommandResult<()> {
    let path = state.paths.config_dir().join(DISCLAIMER_MARKER);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, b"ok").map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Workspaces
// ---------------------------------------------------------------------------

fn workspace_manager(state: &AppState) -> WorkspaceManager {
    WorkspaceManager::new(state.paths.workspaces_dir())
}

#[tauri::command]
pub fn list_workspaces(state: State<'_, AppState>) -> CommandResult<Vec<Workspace>> {
    workspace_manager(&state).list().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_workspace(
    state: State<'_, AppState>,
    name: String,
    specialty: Option<String>,
    language: Option<String>,
) -> CommandResult<Workspace> {
    workspace_manager(&state)
        .create(&name, specialty, language)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn switch_workspace(
    state: State<'_, AppState>,
    id_or_name: String,
) -> CommandResult<Workspace> {
    let ws = workspace_manager(&state)
        .load(&id_or_name)
        .map_err(|e| e.to_string())?;
    let mut cfg = state.config.lock().map_err(|_| "config poisoned")?.clone();
    cfg.general.default_workspace.clone_from(&ws.id);
    cfg.save(state.paths.config_file())
        .map_err(|e| e.to_string())?;
    *state.config.lock().map_err(|_| "config poisoned")? = cfg;
    Ok(ws)
}

#[tauri::command]
pub fn active_workspace(state: State<'_, AppState>) -> CommandResult<Option<Workspace>> {
    let name = state
        .config
        .lock()
        .map_err(|_| "config poisoned")?
        .general
        .default_workspace
        .clone();
    if name.is_empty() {
        return Ok(None);
    }
    match workspace_manager(&state).load(&name) {
        Ok(ws) => Ok(Some(ws)),
        Err(_) => Ok(None),
    }
}

#[tauri::command]
pub async fn delete_workspace(state: State<'_, AppState>, id_or_name: String) -> CommandResult<()> {
    workspace_manager(&state)
        .delete(&id_or_name)
        .map_err(|e| e.to_string())?;
    // Drop the cached repository handle so the next workspace with the same id
    // (after re-creation) opens a fresh SQLite/LanceDB connection instead of
    // reusing the now-deleted directory.
    state.repos.lock().await.remove(&id_or_name);
    Ok(())
}

// ---------------------------------------------------------------------------
// Documents (knowledge base)
// ---------------------------------------------------------------------------

/// Return a repository handle for `workspace_id`, opening it the first time
/// and caching the `Arc` for subsequent calls. SQLite + LanceDB setup is the
/// second-most-expensive thing after the embedder, so we hold them open.
async fn get_repo(state: &AppState, workspace_id: &str) -> Result<Arc<DocumentRepository>, String> {
    let mut cache = state.repos.lock().await;
    if let Some(repo) = cache.get(workspace_id) {
        return Ok(Arc::clone(repo));
    }
    let dir = state.paths.workspace_dir(workspace_id);
    let layout = RepositoryLayout::new(dir);
    let repo = Arc::new(
        DocumentRepository::open(layout, state.embedder.dim())
            .await
            .map_err(|e| e.to_string())?,
    );
    cache.insert(workspace_id.to_string(), Arc::clone(&repo));
    Ok(repo)
}

#[tauri::command]
pub async fn list_documents(
    state: State<'_, AppState>,
    workspace_id: String,
) -> CommandResult<Vec<DocumentRecord>> {
    let repo = get_repo(&state, &workspace_id).await?;
    repo.list().map_err(|e| e.to_string())
}

#[derive(Debug, Serialize)]
pub struct DocumentDetail {
    pub record: DocumentRecord,
    pub chunk_count: usize,
    pub sample_text: Option<String>,
}

#[tauri::command]
pub async fn show_document(
    state: State<'_, AppState>,
    workspace_id: String,
    id: String,
) -> CommandResult<Option<DocumentDetail>> {
    let repo = get_repo(&state, &workspace_id).await?;
    match repo.show(&id).map_err(|e| e.to_string())? {
        Some(d) => Ok(Some(DocumentDetail {
            record: d.record,
            chunk_count: d.chunk_count,
            sample_text: d.sample_text,
        })),
        None => Ok(None),
    }
}

#[tauri::command]
pub async fn remove_document(
    state: State<'_, AppState>,
    workspace_id: String,
    id: String,
) -> CommandResult<bool> {
    let repo = get_repo(&state, &workspace_id).await?;
    repo.remove(&id).await.map_err(|e| e.to_string())
}

#[derive(Debug, Serialize)]
pub struct IngestSummary {
    pub ingested: usize,
    pub skipped: usize,
    pub failed: usize,
    pub messages: Vec<String>,
}

/// DTO mirroring `IngestionEvent` for the frontend. We don't put `Serialize`
/// on the rag crate's event type to keep that crate frontend-agnostic.
#[derive(Debug, Serialize, Clone)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum IngestionEventDto {
    Starting {
        path: String,
    },
    Progress {
        path: String,
        stage: ProgressStage,
        percent: u8,
    },
    Ingested {
        path: String,
        doc_id: String,
    },
    Skipped {
        path: String,
        reason: String,
    },
    Failed {
        path: String,
        error: String,
    },
}

impl IngestionEventDto {
    fn from_event(ev: &IngestionEvent) -> Self {
        match ev {
            IngestionEvent::Starting(p) => Self::Starting {
                path: p.display().to_string(),
            },
            IngestionEvent::Progress {
                path,
                stage,
                percent,
            } => Self::Progress {
                path: path.display().to_string(),
                stage: *stage,
                percent: *percent,
            },
            IngestionEvent::Ingested { path, record } => Self::Ingested {
                path: path.display().to_string(),
                doc_id: record.id.clone(),
            },
            IngestionEvent::Skipped { path, reason } => Self::Skipped {
                path: path.display().to_string(),
                reason: match reason {
                    SkipReason::UnsupportedType => "unsupported_type".to_string(),
                    SkipReason::NeedsOcr => "needs_ocr".to_string(),
                },
            },
            IngestionEvent::Failed { path, error } => Self::Failed {
                path: path.display().to_string(),
                error: error.clone(),
            },
        }
    }
}

#[tauri::command]
pub async fn ingest_path(
    state: State<'_, AppState>,
    workspace_id: String,
    path: String,
) -> CommandResult<IngestSummary> {
    let embedder = Arc::clone(&state.embedder);
    let repo = get_repo(&state, &workspace_id).await?;
    let cfg = state.config.lock().map_err(|_| "config poisoned")?.clone();
    let chunk_params = ChunkParams::new(
        cfg.rag.chunk_size,
        cfg.rag
            .chunk_size
            .saturating_sub(cfg.rag.chunk_overlap)
            .max(1),
        cfg.rag.chunk_overlap,
    )
    .map_err(|e| e.to_string())?;
    let pipeline =
        IngestionPipeline::new(embedder, repo, chunk_params).map_err(|e| e.to_string())?;
    let mut messages = Vec::new();
    let report = pipeline
        .ingest_path(std::path::Path::new(&path), |ev| match ev {
            IngestionEvent::Starting(p) => messages.push(format!("→ {}", p.display())),
            IngestionEvent::Ingested { path, record } => {
                messages.push(format!("✓ {} → {}", path.display(), record.id));
            }
            IngestionEvent::Skipped { path, reason } => {
                messages.push(format!("· {} skipped: {reason:?}", path.display()));
            }
            IngestionEvent::Failed { path, error } => {
                messages.push(format!("✗ {} failed: {error}", path.display()));
            }
            IngestionEvent::Progress { .. } => {}
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(IngestSummary {
        ingested: report.ingested.len(),
        skipped: report.skipped.len(),
        failed: report.failed.len(),
        messages,
    })
}

/// Multi-file ingestion with parallel processing and streaming progress
/// events to the frontend over the `ingest:progress` Tauri event.
///
/// Up to four files are processed concurrently; the embedder's internal
/// ONNX inference mutex naturally serializes the GPU/CPU-bound step so this
/// is safe and gives a measurable wall-clock win on the extract/store phases.
#[tauri::command]
pub async fn ingest_paths(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    workspace_id: String,
    paths: Vec<String>,
) -> CommandResult<IngestSummary> {
    let embedder = Arc::clone(&state.embedder);
    let repo = get_repo(&state, &workspace_id).await?;
    let cfg = state.config.lock().map_err(|_| "config poisoned")?.clone();
    let chunk_params = ChunkParams::new(
        cfg.rag.chunk_size,
        cfg.rag
            .chunk_size
            .saturating_sub(cfg.rag.chunk_overlap)
            .max(1),
        cfg.rag.chunk_overlap,
    )
    .map_err(|e| e.to_string())?;

    state.ingest_cancel.store(false, Ordering::SeqCst);
    let cancel = Arc::clone(&state.ingest_cancel);

    let pipeline =
        Arc::new(IngestionPipeline::new(embedder, repo, chunk_params).map_err(|e| e.to_string())?);
    let semaphore = Arc::new(tokio::sync::Semaphore::new(4));
    let mut handles = Vec::with_capacity(paths.len());

    for path in paths {
        let pipeline = Arc::clone(&pipeline);
        let semaphore = Arc::clone(&semaphore);
        let app = app.clone();
        let cancel = Arc::clone(&cancel);
        handles.push(tokio::spawn(async move {
            let Ok(_permit) = semaphore.acquire().await else {
                return None;
            };
            if cancel.load(Ordering::SeqCst) {
                return None;
            }
            let p = std::path::PathBuf::from(&path);
            let mut local_msgs: Vec<String> = Vec::new();
            let app_for_cb = app.clone();
            let result = pipeline
                .ingest_path(&p, |ev| {
                    let _ = app_for_cb.emit("ingest:progress", IngestionEventDto::from_event(&ev));
                    match &ev {
                        IngestionEvent::Ingested { path, record } => {
                            local_msgs.push(format!("✓ {} → {}", path.display(), record.id));
                        }
                        IngestionEvent::Skipped { path, reason } => {
                            local_msgs.push(format!("· {} skipped: {reason:?}", path.display()));
                        }
                        IngestionEvent::Failed { path, error } => {
                            local_msgs.push(format!("✗ {} failed: {error}", path.display()));
                        }
                        _ => {}
                    }
                })
                .await;
            match result {
                Ok(report) => Some((report, local_msgs)),
                Err(e) => {
                    let _ = app.emit(
                        "ingest:progress",
                        IngestionEventDto::Failed {
                            path: p.display().to_string(),
                            error: e.to_string(),
                        },
                    );
                    None
                }
            }
        }));
    }

    let mut summary = IngestSummary {
        ingested: 0,
        skipped: 0,
        failed: 0,
        messages: Vec::new(),
    };
    for h in handles {
        if let Ok(Some((report, msgs))) = h.await {
            summary.ingested += report.ingested.len();
            summary.skipped += report.skipped.len();
            summary.failed += report.failed.len();
            summary.messages.extend(msgs);
        }
    }
    Ok(summary)
}

#[tauri::command]
pub fn ingest_cancel(state: State<'_, AppState>) -> CommandResult<()> {
    state.ingest_cancel.store(true, Ordering::SeqCst);
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct AskDocumentsRequest {
    pub workspace_id: String,
    pub question: String,
    pub provider_id: String,
    pub model: Option<String>,
    /// If `true`, when the workspace documents don't cover the question the
    /// model may answer from its general training knowledge — but it MUST
    /// flag the answer as such (the system prompt forces the disclosure).
    /// No live web access is involved in either mode.
    #[serde(default)]
    pub allow_general_knowledge: bool,
}

#[derive(Debug, Serialize)]
pub struct QaSourceDto {
    pub index: usize,
    pub document_id: String,
    pub document_title: String,
    pub chunk_id: String,
    pub snippet: String,
}

#[derive(Debug, Serialize)]
pub struct WebSourceDto {
    pub url: String,
    pub title: String,
    pub snippet: String,
}

#[derive(Debug, Serialize)]
pub struct AskDocumentsResponse {
    pub answer: String,
    pub sources: Vec<QaSourceDto>,
    pub web_sources: Vec<WebSourceDto>,
    pub model: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[tauri::command]
pub async fn ask_documents(
    state: State<'_, AppState>,
    request: AskDocumentsRequest,
) -> CommandResult<AskDocumentsResponse> {
    let workspace = workspace_manager(&state)
        .load(&request.workspace_id)
        .map_err(|e| e.to_string())?;
    // Local CLI providers (`claude-cli`, `codex-cli`) authenticate via
    // the user's own CLI install — no Conclave-side keychain entry to
    // look up. Treat them like the OAuth providers and pass an empty
    // key to `build_provider`. Without this branch the request fails
    // up-front with "no API key for codex-cli" before the deliberation
    // ever starts, which to the user looks like a silent no-op.
    let api_key = match request.provider_id.as_str() {
        "ollama" | "apple-intelligence" | "anthropic-oauth" | "openai-oauth" | "claude-cli"
        | "codex-cli" => String::new(),
        other => secrets::load(other)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("no API key for {other}"))?,
    };
    let provider = build_provider(
        &request.provider_id,
        &api_key,
        request.model.clone(),
        state.paths.config_dir(),
    )?;
    ensure_general_scope(&provider)?;
    ensure_provider_ready(&provider).await?;
    let embedder = Arc::clone(&state.embedder);
    let repo = get_repo(&state, &request.workspace_id).await?;
    let top_k = state
        .config
        .lock()
        .map_err(|_| "config poisoned")?
        .rag
        .top_k;
    let lang = workspace.language.clone().unwrap_or_else(|| "es".into());
    let pipeline = QaPipeline::new(embedder, repo, provider);
    let r = pipeline
        .ask(
            &request.question,
            top_k,
            &lang,
            request.allow_general_knowledge,
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(AskDocumentsResponse {
        answer: r.answer,
        web_sources: r
            .web_sources
            .into_iter()
            .map(|w| WebSourceDto {
                url: w.url,
                title: w.title,
                snippet: w.snippet,
            })
            .collect(),
        sources: r
            .sources
            .into_iter()
            .map(|s| QaSourceDto {
                index: s.index,
                document_id: s.document_id,
                document_title: s.document_title,
                chunk_id: s.chunk_id,
                snippet: s.snippet,
            })
            .collect(),
        model: r.model,
        input_tokens: r.input_tokens,
        output_tokens: r.output_tokens,
    })
}

// ---------------------------------------------------------------------------
// Deident
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct DeidentResponse {
    pub masked_text: String,
    pub span_count: usize,
    pub strict_clean: bool,
}

#[tauri::command]
pub fn deident_text(text: String) -> CommandResult<DeidentResponse> {
    let pipeline = PipelineDeidentifier::new();
    let result = pipeline.deidentify(&text).map_err(|e| e.to_string())?;
    Ok(DeidentResponse {
        masked_text: result.masked_text,
        span_count: result.spans.len(),
        strict_clean: result.strict_mode_clean,
    })
}

// ---------------------------------------------------------------------------
// Providers
// ---------------------------------------------------------------------------

/// Unified state machine for every provider entry the UI renders. Replaces
/// the older `configured` + `available` pair, which let us tell the UI
/// "connected AND reachable" purely because a credentials file existed on
/// disk — without ever validating that the credential still worked. The
/// six variants below are mutually exclusive; the frontend renders a
/// single status pill per provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderStatus {
    /// Credential is present and (for OAuth/Ollama) the upstream probe
    /// succeeded. API-key providers map to `Ready` purely on key
    /// presence — see `list_providers` for the asymmetry rationale.
    Ready,
    /// OAuth session is no longer accepted by the provider (401/403 on
    /// probe, or refresh-token rejected). Distinct from `Unreachable`
    /// because the fix is reconnect / switch to API key, not retry.
    Expired,
    /// Transport-level failure on the probe — network down, DNS timeout,
    /// provider 5xx. Retrying may succeed.
    Unreachable,
    /// No credential present (no API key in keychain, no OAuth file on
    /// disk). The user must run through the connect flow first.
    NotConfigured,
    /// CLI binary is installed on `$PATH` but the user hasn't completed
    /// the CLI's own login. Tied to the CLI provider ids only.
    LoginRequired,
    /// CLI binary is not on `$PATH`. Tied to the CLI provider ids only.
    NotInstalled,
}

impl ProviderStatus {
    /// `true` for the one status that allows actually calling the
    /// provider. Mirrors the old `configured && available` check the
    /// frontend used to do at every call site.
    pub fn is_ready(self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// How long a probe result stays in the cache before we hit the upstream
/// provider again. Picked to balance "UI feels responsive" against "stop
/// burning quota on every render". A user hammering the Refresh button
/// can still force a fresh probe via `force_refresh: true`.
const PROBE_TTL: Duration = Duration::from_secs(60);

/// Upper bound for a single probe call. The Codex and Anthropic
/// endpoints normally respond well under a second; anything past 4s
/// we treat as `Unreachable` so a hung connection can't block the
/// Settings page from rendering.
const PROBE_TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Debug, Serialize)]
pub struct ProviderInfo {
    pub id: String,
    pub status: ProviderStatus,
    pub default_model: String,
    pub requires_network: bool,
    pub auth: String,
    pub kind: String,
    pub hint: Option<String>,
}

/// Build the `ProviderInfo` entry for Apple Intelligence — or return
/// `None` when the host can't run it.
///
/// We surface the provider only when the user has a plausible path to
/// using it: the model is ready, Apple Intelligence is toggled off in
/// System Settings, or the on-device model is still downloading.
/// Devices that are structurally ineligible (Intel Macs, macOS < 26,
/// frameworks missing) get `None` so the Settings card and every
/// downstream picker simply skip the entry.
async fn apple_intelligence_info() -> Option<ProviderInfo> {
    let availability = AppleIntelligenceProvider::new().availability().await;
    if !availability.is_user_actionable() {
        return None;
    }
    let available = matches!(availability, AppleIntelligenceAvailability::Available);
    Some(ProviderInfo {
        id: "apple-intelligence".to_owned(),
        // No keychain entry to consult — the runtime availability is
        // the only "is this ready?" signal. Mirrors how Ollama is
        // surfaced (always present, ready == reachable). When the
        // model is still downloading or AI is off, we surface
        // `Unreachable` so the UI's single pill reflects "you can't
        // use this right now" without inventing a new state.
        status: if available {
            ProviderStatus::Ready
        } else {
            ProviderStatus::Unreachable
        },
        default_model: APPLE_INTELLIGENCE_MODEL_LABEL.to_owned(),
        requires_network: false,
        auth: "local".into(),
        kind: "subtask".into(),
        // Stable tag the frontend maps to its i18n string ("not_enabled"
        // → "Turn on Apple Intelligence in System Settings", etc.).
        hint: if available {
            None
        } else {
            Some(availability.tag().to_owned())
        },
    })
}

/// Probe an OAuth provider's stored credentials by calling the real
/// upstream endpoint (`probe()`). Wrapped in a 4-second timeout so a
/// hung TLS handshake can't block the entire Settings render.
async fn probe_oauth_status(id: &str, path: &std::path::Path) -> ProviderStatus {
    use conclave_providers::ProviderError;
    let probe_fut = async {
        match id {
            "anthropic-oauth" => match AnthropicOAuthProvider::from_conclave_tokens(path) {
                Ok(p) => p.probe().await,
                Err(e) => Err(e),
            },
            "openai-oauth" => match OpenAIOAuthProvider::from_conclave_tokens(path) {
                Ok(p) => p.probe().await,
                Err(e) => Err(e),
            },
            _ => Err(ProviderError::Other(format!("unknown oauth id `{id}`"))),
        }
    };
    match tokio::time::timeout(PROBE_TIMEOUT, probe_fut).await {
        Ok(Ok(())) => ProviderStatus::Ready,
        Ok(Err(ProviderError::Auth)) => ProviderStatus::Expired,
        Ok(Err(_)) | Err(_) => ProviderStatus::Unreachable,
    }
}

#[tauri::command]
pub async fn list_providers(
    state: State<'_, AppState>,
    force_refresh: Option<bool>,
) -> CommandResult<Vec<ProviderInfo>> {
    let force = force_refresh.unwrap_or(false);
    let mut out = Vec::new();
    for id in KNOWN_PROVIDERS {
        // Apple Intelligence is omitted entirely on hosts where it
        // is structurally unreachable (Intel Mac, macOS < 26, etc.).
        // We surface it only when the user can plausibly turn it on
        // or wait for the model download — see
        // `AppleIntelligenceAvailability::is_user_actionable`.
        if *id == "apple-intelligence" {
            if let Some(info) = apple_intelligence_info().await {
                out.push(info);
            }
            continue;
        }

        let configured = secrets::load(id).unwrap_or(None).is_some();
        let (status, default_model, requires_net) = match *id {
            "ollama" => {
                // Local Ollama daemon — `ping()` is cheap and bounded
                // by reqwest's default connect timeout. No keychain
                // entry to consult; the daemon's availability IS the
                // "ready" signal.
                let p = OllamaProvider::new();
                let reachable = p.ping().await;
                (
                    if reachable {
                        ProviderStatus::Ready
                    } else {
                        ProviderStatus::Unreachable
                    },
                    "llama3.1:8b".to_owned(),
                    false,
                )
            }
            "anthropic" => (
                if configured {
                    ProviderStatus::Ready
                } else {
                    ProviderStatus::NotConfigured
                },
                "claude-sonnet-4-6-20250929".into(),
                true,
            ),
            "openai" => (
                if configured {
                    ProviderStatus::Ready
                } else {
                    ProviderStatus::NotConfigured
                },
                "gpt-5".into(),
                true,
            ),
            "openrouter" => (
                if configured {
                    ProviderStatus::Ready
                } else {
                    ProviderStatus::NotConfigured
                },
                "set per call".into(),
                true,
            ),
            _ => (ProviderStatus::NotConfigured, "—".into(), false),
        };
        out.push(ProviderInfo {
            id: (*id).to_owned(),
            status,
            default_model,
            requires_network: requires_net,
            auth: if *id == "ollama" {
                "local".into()
            } else {
                "api-key".into()
            },
            kind: "standard".into(),
            hint: None,
        });
    }
    for id in OAUTH_PROVIDERS {
        // "Configured" requires an explicit in-app sign-in. We deliberately
        // do NOT fall back to `~/.codex/auth.json` or `~/.claude/.credentials.json`
        // — picking those up silently confuses users who never connected the
        // provider in Conclave but happen to have the CLI installed.
        let conclave_path = state
            .paths
            .config_dir()
            .join("oauth")
            .join(format!("{id}.json"));
        let (status, hint) = if conclave_path.exists() {
            // Read the cache before probing — repeated UI refreshes
            // (Settings auto-refresh, title-bar polling) all share
            // the same 60s window.
            let cached = if force {
                None
            } else {
                let guard = state.probe_cache.lock().await;
                guard
                    .get(*id)
                    .filter(|(when, _)| when.elapsed() < PROBE_TTL)
                    .map(|(_, s)| *s)
            };
            let probed = if let Some(s) = cached {
                s
            } else {
                let s = probe_oauth_status(id, &conclave_path).await;
                let mut guard = state.probe_cache.lock().await;
                guard.insert((*id).to_owned(), (Instant::now(), s));
                s
            };
            let hint = match *id {
                "anthropic-oauth" => AnthropicOAuthProvider::from_conclave_tokens(&conclave_path)
                    .ok()
                    .and_then(|p| p.subscription_type()),
                "openai-oauth" => OpenAIOAuthProvider::from_conclave_tokens(&conclave_path)
                    .ok()
                    .and_then(|p| p.account_label()),
                _ => None,
            };
            (probed, hint)
        } else {
            // No credential file — clear any stale cache entry so a
            // fresh sign-in is reflected immediately.
            let mut guard = state.probe_cache.lock().await;
            guard.remove(*id);
            (
                ProviderStatus::NotConfigured,
                Some("sign in to start".into()),
            )
        };
        let default_model = match *id {
            "anthropic-oauth" => "claude-sonnet-4-6-20250929".into(),
            "openai-oauth" => "gpt-5.5".into(),
            _ => "—".into(),
        };
        out.push(ProviderInfo {
            id: (*id).to_owned(),
            status,
            default_model,
            requires_network: true,
            auth: "oauth".into(),
            kind: "oauth".into(),
            hint,
        });
    }
    // Local CLI providers. Maps the two underlying flags
    // (`is_installed` / `is_logged_in`) onto the three statuses the UI
    // cares about — `NotInstalled` (binary missing), `LoginRequired`
    // (binary present but no session), `Ready`. The `hint` carries a
    // stable tag the frontend i18n maps to actionable copy ("install
    // from claude.com/code", "run `codex login`", etc.).
    //
    // The "user explicitly disconnected" set lives in `conclave.toml`
    // (`providers.disabled_provider_ids`). When a CLI id appears
    // there we surface it as `NotConfigured` even though the binary
    // is still installed + logged in via its own credentials — that's
    // what "Disconnect" must mean for a provider Conclave does not
    // own credentials for. Re-picking the tile clears the flag.
    let (disabled_ids, cli_overrides): (Vec<String>, std::collections::HashMap<String, bool>) = {
        let guard = state.config.lock().map_err(|_| "config poisoned")?;
        (
            guard.providers.disabled_provider_ids.clone(),
            guard.providers.cli_local_overrides.clone(),
        )
    };
    // Force-refresh on Reload (or after the frontend's redetect button)
    // invalidates the binary path cache so the very next probe re-walks
    // `$PATH`. Without this the user would have to relaunch Conclave to
    // pick up a CLI they just installed in another window.
    if force {
        ClaudeCliProvider::refresh_binary_cache();
        CodexCliProvider::refresh_binary_cache();
    }
    for id in CLI_PROVIDERS {
        let (status, default_model) = match *id {
            "claude-cli" => {
                let installed = ClaudeCliProvider::is_installed();
                let logged_in = if installed {
                    ClaudeCliProvider::is_logged_in().await
                } else {
                    false
                };
                let status = if !installed {
                    ProviderStatus::NotInstalled
                } else if !logged_in {
                    ProviderStatus::LoginRequired
                } else {
                    ProviderStatus::Ready
                };
                (status, CLAUDE_CLI_DEFAULT_MODEL.to_owned())
            }
            "codex-cli" => {
                let installed = CodexCliProvider::is_installed();
                let logged_in = if installed {
                    CodexCliProvider::is_logged_in().await
                } else {
                    false
                };
                let status = if !installed {
                    ProviderStatus::NotInstalled
                } else if !logged_in {
                    ProviderStatus::LoginRequired
                } else {
                    ProviderStatus::Ready
                };
                (status, CODEX_CLI_DEFAULT_MODEL.to_owned())
            }
            _ => continue,
        };
        // Manual override: when the user told us "I'm logged in, trust
        // me", upgrade `LoginRequired` to `Ready` as long as the binary
        // is still on PATH. We never override `NotInstalled` — that
        // would just mask a real problem.
        let user_override = matches!(cli_overrides.get(*id), Some(true));
        let promoted_status = if user_override && matches!(status, ProviderStatus::LoginRequired) {
            ProviderStatus::Ready
        } else {
            status
        };
        let user_disabled = disabled_ids.iter().any(|d| d == *id);
        // Override only when the user disconnected from inside Conclave
        // AND the CLI is otherwise usable — leave `NotInstalled` /
        // `LoginRequired` alone so the picker still surfaces the
        // actionable install/login hint.
        let final_status = if user_disabled && matches!(promoted_status, ProviderStatus::Ready) {
            ProviderStatus::NotConfigured
        } else {
            promoted_status
        };
        let hint = match final_status {
            ProviderStatus::NotInstalled => Some("not_installed".into()),
            ProviderStatus::LoginRequired => Some("login_required".into()),
            ProviderStatus::Ready if user_override => Some("user_marked_ready".into()),
            _ => None,
        };
        out.push(ProviderInfo {
            id: (*id).to_owned(),
            status: final_status,
            default_model,
            requires_network: true,
            auth: "cli".into(),
            kind: "cli".into(),
            hint,
        });
    }
    Ok(out)
}

/// Mutate the `providers.disabled_provider_ids` set and persist
/// `conclave.toml`. Used by `set_provider_key` / `remove_provider_key`
/// to record "the user has explicitly disconnected this CLI inside
/// Conclave" without touching the user's actual CLI credentials.
fn set_cli_disabled(state: &AppState, id: &str, disabled: bool) -> CommandResult<()> {
    let mut cfg = state.config.lock().map_err(|_| "config poisoned")?.clone();
    let already_disabled = cfg.providers.disabled_provider_ids.iter().any(|d| d == id);
    if disabled == already_disabled {
        return Ok(());
    }
    if disabled {
        cfg.providers.disabled_provider_ids.push(id.to_owned());
    } else {
        cfg.providers.disabled_provider_ids.retain(|d| d != id);
    }
    cfg.save(state.paths.config_file())
        .map_err(|e| e.to_string())?;
    *state.config.lock().map_err(|_| "config poisoned")? = cfg;
    Ok(())
}

#[tauri::command]
pub async fn set_provider_key(
    state: State<'_, AppState>,
    id: String,
    api_key: String,
) -> CommandResult<()> {
    if matches!(id.as_str(), "claude-cli" | "codex-cli") {
        // CLI providers have no Conclave-side credential to store
        // (auth is handled by the user's installed CLI), so the
        // picker tile click reuses this command to clear the
        // user-disabled flag and resurface the CLI as the active
        // provider. The `api_key` argument is ignored.
        let _ = api_key;
        return set_cli_disabled(&state, &id, false);
    }
    if matches!(id.as_str(), "ollama" | "apple-intelligence") {
        // Always-available local providers — nothing to persist.
        return ok(());
    }
    if api_key.trim().is_empty() {
        return err("API key cannot be empty");
    }
    secrets::store(&id, &api_key).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn test_provider(
    state: State<'_, AppState>,
    id: String,
    prompt: Option<String>,
) -> CommandResult<String> {
    let api_key = match id.as_str() {
        "ollama" | "apple-intelligence" | "anthropic-oauth" | "openai-oauth" | "claude-cli"
        | "codex-cli" => String::new(),
        _ => match secrets::load(&id).map_err(|e| e.to_string())? {
            Some(k) => k,
            None => return err(format!("no API key for {id}")),
        },
    };
    let provider = build_provider(&id, &api_key, None, state.paths.config_dir())?;
    let prompt = prompt.unwrap_or_else(|| "Reply with one word: hello.".into());
    let result = provider
        .complete(CompletionRequest {
            model: String::new(),
            messages: vec![Message::user(prompt)],
            max_output_tokens: Some(50),
            temperature: Some(0.0),
            json_schema: None,
            allow_web_search: false,
            images: Vec::new(),
        })
        .await;
    // The test is the same code path the committee uses, so the
    // result tells us truthfully whether the provider is currently
    // healthy. Update the probe cache so the next `list_providers`
    // (which the UI fires right after the test) reflects the new
    // status without waiting for the TTL to expire. Only OAuth ids
    // are cached today — the others compute status cheaply enough
    // that there's no cache entry to invalidate.
    if matches!(id.as_str(), "anthropic-oauth" | "openai-oauth") {
        use conclave_providers::ProviderError;
        let cached_status = match &result {
            Ok(_) => Some(ProviderStatus::Ready),
            Err(ProviderError::Auth) => Some(ProviderStatus::Expired),
            Err(
                ProviderError::Network(_)
                | ProviderError::Unavailable(_)
                | ProviderError::RateLimit { .. },
            ) => Some(ProviderStatus::Unreachable),
            Err(_) => None,
        };
        if let Some(s) = cached_status {
            let mut guard = state.probe_cache.lock().await;
            guard.insert(id.clone(), (Instant::now(), s));
        }
    }
    let resp = result.map_err(|e| e.to_string())?;
    Ok(format!(
        "{}\n\n— {} ({}+{} tokens)",
        resp.text, resp.model, resp.usage.input_tokens, resp.usage.output_tokens
    ))
}

#[tauri::command]
pub fn remove_provider_key(state: State<'_, AppState>, id: String) -> CommandResult<()> {
    if matches!(id.as_str(), "claude-cli" | "codex-cli") {
        // No Conclave-managed credential to delete (auth lives in
        // the user's installed CLI). Persist a "user disconnected
        // this CLI inside Conclave" flag instead so the next
        // `list_providers` returns `NotConfigured` and the picker
        // re-appears. The user's actual CLI session is left alone —
        // re-picking the tile clears the flag.
        return set_cli_disabled(&state, &id, true);
    }
    secrets::delete(&id).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// OAuth subscription login (Phase 2.5)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct OAuthStartResponse {
    pub url: String,
    pub provider_id: String,
    pub instructions: String,
}

#[tauri::command]
pub async fn oauth_anthropic_start(
    state: State<'_, AppState>,
) -> CommandResult<OAuthStartResponse> {
    let started = AnthropicLoginFlow::start().map_err(|e| e.to_string())?;
    let _ = open_in_browser(&started.url);
    {
        let mut guard = state
            .anthropic_login
            .lock()
            .map_err(|_| "anthropic login mutex poisoned".to_string())?;
        *guard = Some(started.flow);
    }
    Ok(OAuthStartResponse {
        url: started.url,
        provider_id: "anthropic-oauth".into(),
        instructions: "Anthropic will show you a one-time code on the callback page. \
            Paste it back here to finish signing in."
            .into(),
    })
}

#[tauri::command]
pub async fn oauth_anthropic_complete(
    state: State<'_, AppState>,
    code: String,
) -> CommandResult<()> {
    let flow = {
        let mut guard = state
            .anthropic_login
            .lock()
            .map_err(|_| "anthropic login mutex poisoned".to_string())?;
        guard
            .take()
            .ok_or_else(|| "no active Anthropic login — click Sign in first".to_string())?
    };
    let tokens = flow.complete(&code).await.map_err(|e| e.to_string())?;
    persist_tokens(state.paths.config_dir(), "anthropic-oauth", &tokens)
        .map_err(|e| e.to_string())?;
    // Drop any stale probe result from a previous session so the
    // next `list_providers` call triggers a fresh probe against the
    // brand-new credentials.
    state.probe_cache.lock().await.remove("anthropic-oauth");
    Ok(())
}

/// Begin an OpenAI OAuth login.
///
/// Spawns a background task that owns the localhost:1455 listener and waits
/// up to 5 min for the browser to redirect. Tries to open the URL in the
/// default browser, but also returns it so the UI can offer "copy" and
/// "open" affordances — auth.openai.com sometimes drops the OAuth state
/// when the user has to log in from cold (browser shows "session ended"),
/// and the only reliable recovery is to open the URL in a tab where the
/// ChatGPT session is already live.
#[tauri::command]
pub async fn oauth_openai_start(state: State<'_, AppState>) -> CommandResult<OAuthStartResponse> {
    // Cancel any previous in-flight flow so its listener gets dropped
    // before we try to bind. The drop happens on the runtime's next tick,
    // so we yield and briefly sleep to give the socket a chance to release.
    {
        let mut guard = state.openai_login.lock().map_err(|_| "state poisoned")?;
        if let Some(handle) = guard.take() {
            handle.abort();
        }
    }
    tokio::task::yield_now().await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let started = OpenAILoginFlow::start().await.map_err(|e| e.to_string())?;
    let url = started.url.clone();
    let _ = open_in_browser(&url);

    let config_dir = state.paths.config_dir().to_owned();
    let probe_cache = Arc::clone(&state.probe_cache);
    let task = tokio::spawn(async move {
        match started
            .flow
            .wait_for_callback(std::time::Duration::from_secs(300))
            .await
        {
            Ok(tokens) => {
                if let Err(e) = persist_tokens(&config_dir, "openai-oauth", &tokens) {
                    eprintln!("openai oauth persist failed: {e}");
                }
                // Clear any cached probe outcome from a previous
                // session — the next `list_providers` poll will
                // exercise the brand-new credentials directly.
                probe_cache.lock().await.remove("openai-oauth");
            }
            Err(e) => {
                eprintln!("openai oauth flow failed: {e}");
            }
        }
    });

    *state.openai_login.lock().map_err(|_| "state poisoned")? = Some(task.abort_handle());

    Ok(OAuthStartResponse {
        url,
        provider_id: "openai-oauth".into(),
        instructions: String::new(),
    })
}

/// Abort an in-flight OpenAI OAuth flow. Releases the localhost:1455
/// listener so the next attempt can bind. No-op if no flow is in progress
/// or the flow has already completed.
#[tauri::command]
pub fn oauth_openai_cancel(state: State<'_, AppState>) -> CommandResult<()> {
    let mut guard = state.openai_login.lock().map_err(|_| "state poisoned")?;
    if let Some(handle) = guard.take() {
        handle.abort();
    }
    Ok(())
}

#[tauri::command]
pub async fn oauth_logout(state: State<'_, AppState>, id: String) -> CommandResult<()> {
    let path = state
        .paths
        .config_dir()
        .join("oauth")
        .join(format!("{id}.json"));
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    // Always clear any cached probe result so the next
    // `list_providers` reflects the disconnected state.
    state.probe_cache.lock().await.remove(&id);
    Ok(())
}

fn build_provider(
    id: &str,
    api_key: &str,
    model: Option<String>,
    config_dir: &std::path::Path,
) -> Result<Arc<dyn LlmProvider>, String> {
    Ok(match id {
        "anthropic" => {
            let mut p = AnthropicProvider::new(api_key.to_owned());
            if let Some(m) = model {
                p = p.with_model(m);
            }
            Arc::new(p)
        }
        "openai" => {
            let mut p = OpenAiProvider::new(api_key.to_owned());
            if let Some(m) = model {
                p = p.with_model(m);
            }
            Arc::new(p)
        }
        "openrouter" => {
            let mut p = OpenRouterProvider::new(api_key.to_owned());
            p = match model {
                Some(m) => p.with_model(m),
                None => p.with_model("anthropic/claude-3.5-sonnet"),
            };
            Arc::new(p)
        }
        "ollama" => {
            let mut p = OllamaProvider::new();
            if let Some(m) = model {
                p = p.with_model(m);
            }
            Arc::new(p)
        }
        // On-device Apple Intelligence. No credentials, no model
        // selection — `FoundationModels` picks the model itself.
        "apple-intelligence" => Arc::new(AppleIntelligenceProvider::new()),
        // Local CLI providers — shell out to the user's installed
        // `claude` / `codex` binary. Authentication is whatever the
        // user has set up in their own CLI; Conclave never touches
        // those credentials.
        "claude-cli" => {
            let mut p = ClaudeCliProvider::new();
            if let Some(m) = model {
                p = p.with_model(m);
            }
            Arc::new(p)
        }
        "codex-cli" => {
            // Codex `exec` does not document a `--model` flag, so the
            // CLI's own `~/.codex/config.toml` selection wins. The
            // `model` argument here is accepted for API symmetry but
            // not threaded through.
            let _ = model;
            Arc::new(CodexCliProvider::new())
        }
        // OAuth providers read only Conclave's own token file. We do NOT
        // fall back to `~/.codex/auth.json` / `~/.claude/.credentials.json`
        // — see `list_providers` for the rationale.
        "anthropic-oauth" => {
            let path = config_dir.join("oauth").join("anthropic-oauth.json");
            let mut p = AnthropicOAuthProvider::from_conclave_tokens(&path)
                .map_err(|_| "Anthropic is not connected. Sign in from Settings.".to_string())?;
            if let Some(m) = model {
                p = p.with_model(m);
            }
            Arc::new(p)
        }
        "openai-oauth" => {
            let path = config_dir.join("oauth").join("openai-oauth.json");
            let mut p = OpenAIOAuthProvider::from_conclave_tokens(&path)
                .map_err(|_| "OpenAI is not connected. Sign in from Settings.".to_string())?;
            if let Some(m) = model {
                p = p.with_model(m);
            }
            Arc::new(p)
        }
        other => return Err(format!("unknown provider `{other}`")),
    })
}

/// Guard for clinical call sites. Subtask-only providers (e.g. Apple
/// Intelligence, whose vendor guardrails reject clinical content) are
/// barred from deliberation flows. The frontend filters them out of
/// the relevant pickers; this is the backend-side belt-and-braces.
fn ensure_general_scope(provider: &Arc<dyn LlmProvider>) -> CommandResult<()> {
    if provider.capabilities().scope == ProviderScope::Subtask {
        return err(format!(
            "Provider `{}` is restricted to utility tasks and cannot be used for clinical deliberation.",
            provider.id()
        ));
    }
    Ok(())
}

/// Classify an error string returned from a per-case run. When `true`,
/// the failure is structural (transport / unreachable provider) and
/// will keep happening for every other case in the same batch, so the
/// batch runner short-circuits remaining work to `Cancelled` instead
/// of replaying the same timeout N times.
///
/// We match on substrings of the canonical Display messages from
/// `ProviderError` plus the user-facing message from
/// `ensure_provider_ready`. Substring matching is intentional — the
/// errors flow through several `Display`/`format!` layers before
/// landing here as a `String`, so the structured `enum` shape is gone.
fn is_transport_failure(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("network error:")
        || lower.contains("is not responding")
        || lower.contains("provider unavailable:")
        || lower.contains("connection refused")
        // `ProviderError::Auth` Display — a rejected key/token fails
        // every subsequent case the same way, so treat it as
        // structural. (claude-cli's revoked-session error arrives as
        // "provider unavailable:" with its own remediation text.)
        || lower.contains("authentication failed")
}

/// Pre-flight: verify a provider can actually serve a request before
/// committing to a batch or staging drafts. Today only Ollama needs
/// this — the on-device server may be down or unreachable. Cloud
/// providers surface auth/network errors per-call with informative
/// bodies, so a separate ping would only duplicate work for them.
///
/// Returns a single, actionable error string when the local server is
/// unreachable, so the user sees one clear message instead of N
/// identical connection-timeout failures across N cases.
async fn ensure_provider_ready(provider: &Arc<dyn LlmProvider>) -> CommandResult<()> {
    if provider.id() == "ollama" {
        let probe = OllamaProvider::new();
        let ok = tokio::time::timeout(std::time::Duration::from_secs(3), probe.ping())
            .await
            .unwrap_or(false);
        if !ok {
            return err("Ollama is not responding at http://localhost:11434. \
                 Start the server with `ollama serve` or pick another \
                 provider from the selector.");
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Verdict
// ---------------------------------------------------------------------------

fn case_store_arc(state: &AppState, workspace_id: &str) -> Result<Arc<Mutex<CaseStore>>, String> {
    let path = state.paths.workspace_dir(workspace_id).join("cases.sqlite");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let store = CaseStore::open(&path).map_err(|e| e.to_string())?;
    Ok(Arc::new(Mutex::new(store)))
}

#[derive(Debug, Serialize)]
pub struct CaseRunResponse {
    pub case: CaseRecord,
    pub verdict_record: VerdictRecord,
    pub verdict: Verdict,
    /// Attachments persisted alongside this case (empty when none were
    /// provided). Order matches the `[A1..AN]` refs in the verdict.
    pub attachments: Vec<CaseAttachment>,
    pub data_boundary: DataBoundaryPreview,
}

#[derive(Debug, Deserialize)]
pub struct CaseRunRequest {
    pub workspace_id: String,
    pub text: String,
    pub question: String,
    pub provider_id: String,
    pub model: Option<String>,
    /// Absolute paths of files the clinician attached to this case.
    /// Each one is copied under the workspace, de-identified, and made
    /// available to the prompt as a `[A{n}]` evidence block.
    #[serde(default)]
    pub attached_file_paths: Vec<String>,
    /// Optional human-friendly label for the case (e.g. "Juan Pérez" or
    /// "CR-IA-011"). Persists on the case row and is used as the list
    /// title so multi-case batches do not all look identical. Empty falls
    /// back to the question or the case id at render time.
    #[serde(default)]
    pub patient_label: String,
    /// `local_only`, `deid_cloud` (default), or `explicit_phi`.
    #[serde(default)]
    pub data_boundary_mode: Option<String>,
    /// Required when raw PHI payloads (for example cloud vision images)
    /// leave the device.
    #[serde(default)]
    pub allow_phi_payload: bool,
    /// Keep raw narrative locally after a successful run.
    #[serde(default)]
    pub retain_raw_text: bool,
    /// Optional versioned skill overlay.
    #[serde(default)]
    pub active_skill_id: Option<String>,
    /// Opt-in external literature lookup. Sends only a generated
    /// de-identified query, never raw case text.
    #[serde(default)]
    pub use_online_evidence: bool,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize)]
pub struct DataBoundaryPreview {
    pub mode: DataBoundaryMode,
    pub provider_id: String,
    pub provider_requires_network: bool,
    pub sends_masked_text: bool,
    pub sends_raw_text: bool,
    pub sends_images: bool,
    pub stores_raw_text: bool,
    /// Whether the original attachment files will remain on disk after the
    /// run, given the current privacy settings.
    pub retains_attachment_files: bool,
    pub uses_online_evidence: bool,
    pub blocked_reason: Option<String>,
}

fn parse_data_boundary_mode(raw: Option<&str>) -> DataBoundaryMode {
    raw.map(DataBoundaryMode::from_db_str).unwrap_or_default()
}

fn is_image_path(path: &str) -> bool {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "tif" | "tiff" | "heic" | "heif"
    )
}

fn boundary_preview_for_request(
    request: &CaseRunRequest,
    provider: &Arc<dyn LlmProvider>,
    skill_blocked_reason: Option<String>,
    purge_attachments_with_raw_text: bool,
) -> DataBoundaryPreview {
    let mode = parse_data_boundary_mode(request.data_boundary_mode.as_deref());
    let sends_images = request.attached_file_paths.iter().any(|p| is_image_path(p));
    let provider_requires_network = provider.requires_network();
    let boundary_blocked_reason =
        if matches!(mode, DataBoundaryMode::LocalOnly) && provider_requires_network {
            Some("local_only blocks network providers".to_owned())
        } else if matches!(mode, DataBoundaryMode::LocalOnly) && request.use_online_evidence {
            Some("local_only blocks online evidence lookup".to_owned())
        } else if sends_images
            && provider_requires_network
            && !matches!(mode, DataBoundaryMode::ExplicitPhi)
        {
            Some("cloud vision requires explicit_phi mode".to_owned())
        } else if sends_images
            && provider_requires_network
            && matches!(mode, DataBoundaryMode::ExplicitPhi)
            && !request.allow_phi_payload
        {
            Some("explicit_phi mode requires allow_phi_payload consent".to_owned())
        } else {
            None
        };
    let blocked_reason = boundary_blocked_reason.or(skill_blocked_reason);
    DataBoundaryPreview {
        mode,
        provider_id: provider.id().to_owned(),
        provider_requires_network,
        sends_masked_text: true,
        sends_raw_text: false,
        sends_images,
        stores_raw_text: request.retain_raw_text,
        retains_attachment_files: !request.attached_file_paths.is_empty()
            && (request.retain_raw_text || !purge_attachments_with_raw_text),
        uses_online_evidence: request.use_online_evidence,
        blocked_reason,
    }
}

fn enforce_data_boundary(preview: &DataBoundaryPreview) -> CommandResult<()> {
    if let Some(reason) = &preview.blocked_reason {
        return err(reason);
    }
    Ok(())
}

async fn fetch_external_evidence_for_case(
    state: &AppState,
    masked_text: &str,
    question: &str,
) -> CommandResult<Vec<EvidenceItem>> {
    let query = build_external_evidence_query(masked_text, question);
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let cache = Arc::new(
        EvidenceCache::open(state.paths.cache_dir().join("evidence.sqlite"))
            .map_err(|e| e.to_string())?,
    );
    const LIMIT: usize = 5;
    let mut first_error: Option<String> = None;
    if let Ok(email) = std::env::var("CONCLAVE_NCBI_EMAIL") {
        match PubMedSource::new(email).map(|s| s.with_cache(Arc::clone(&cache))) {
            Ok(pubmed) => match pubmed.search(&query, LIMIT).await {
                Ok(items) if !items.is_empty() => return Ok(items),
                Ok(_) => {}
                Err(e) => first_error = Some(e.to_string()),
            },
            Err(e) => first_error = Some(e.to_string()),
        }
    }
    let europe = EuropePmcSource::new()
        .map_err(|e| first_error.clone().unwrap_or_else(|| e.to_string()))?
        .with_cache(cache);
    match europe.search(&query, LIMIT).await {
        Ok(items) => Ok(items),
        Err(e) => Err(first_error.unwrap_or_else(|| e.to_string())),
    }
}

fn build_external_evidence_query(masked_text: &str, question: &str) -> String {
    const STOPWORDS: &[&str] = &[
        "paciente",
        "patient",
        "manejo",
        "management",
        "recomendado",
        "recommended",
        "cuál",
        "cual",
        "what",
        "with",
        "para",
        "por",
        "the",
        "and",
        "los",
        "las",
        "una",
        "uno",
        "del",
        "con",
        "sin",
        "que",
        "está",
        "esta",
        "this",
        "case",
        "años",
        "year",
        "old",
    ];
    let mut terms = Vec::new();
    let combined = format!("{question} {masked_text}");
    let mut current = String::new();
    let mut in_token = false;
    for ch in combined.chars() {
        if ch == '<' {
            current.clear();
            in_token = true;
            continue;
        }
        if in_token {
            if ch == '>' {
                in_token = false;
            }
            continue;
        }
        if ch.is_alphanumeric() || matches!(ch, '-' | '_') {
            current.push(ch.to_ascii_lowercase());
        } else if !current.is_empty() {
            push_query_term(&mut terms, &current, STOPWORDS);
            current.clear();
        }
        if terms.len() >= 10 {
            break;
        }
    }
    if !current.is_empty() && terms.len() < 10 {
        push_query_term(&mut terms, &current, STOPWORDS);
    }
    terms.join(" ")
}

fn push_query_term(out: &mut Vec<String>, term: &str, stopwords: &[&str]) {
    if term.len() < 4 || term.chars().all(|c| c.is_ascii_digit()) {
        return;
    }
    if stopwords.contains(&term) || term.starts_with("patient_name") || term.starts_with("date_") {
        return;
    }
    if !out.iter().any(|t| t == term) {
        out.push(term.to_owned());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivacySettingsDto {
    pub default_data_boundary: DataBoundaryMode,
    pub purge_attachments_with_raw_text: bool,
}

#[tauri::command]
pub fn privacy_settings(state: State<'_, AppState>) -> CommandResult<PrivacySettingsDto> {
    let cfg = state.config.lock().map_err(|_| "config poisoned")?.clone();
    Ok(PrivacySettingsDto {
        default_data_boundary: DataBoundaryMode::from_db_str(&cfg.privacy.default_data_boundary),
        purge_attachments_with_raw_text: cfg.privacy.purge_attachments_with_raw_text,
    })
}

#[tauri::command]
pub fn set_privacy_settings(
    state: State<'_, AppState>,
    settings: PrivacySettingsDto,
) -> CommandResult<PrivacySettingsDto> {
    let mut cfg = state.config.lock().map_err(|_| "config poisoned")?.clone();
    cfg.privacy.default_data_boundary = settings.default_data_boundary.as_db_str().to_owned();
    cfg.privacy.purge_attachments_with_raw_text = settings.purge_attachments_with_raw_text;
    cfg.save(state.paths.config_file())
        .map_err(|e| e.to_string())?;
    *state.config.lock().map_err(|_| "config poisoned")? = cfg;
    Ok(settings)
}

fn load_skill_raw(
    state: &AppState,
    workspace_id: &str,
    skill_id: Option<&str>,
) -> CommandResult<Option<Skill>> {
    let Some(skill_id) = skill_id.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let user_skills = state.paths.config_dir().join("skills");
    let workspace_skills = state.paths.workspace_dir(workspace_id).join("skills");
    conclave_verdict::load_skill(skill_id, Some(&user_skills), Some(&workspace_skills))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("skill `{skill_id}` not found"))
        .map(Some)
}

fn skill_boundary_block_reason(
    state: &AppState,
    workspace_id: &str,
    skill_id: Option<&str>,
    mode: DataBoundaryMode,
) -> Option<String> {
    match load_skill_raw(state, workspace_id, skill_id) {
        Ok(Some(skill)) if !skill.allows_mode(mode) => Some(format!(
            "skill `{}` does not allow data boundary `{}`",
            skill.id,
            mode.as_db_str()
        )),
        Ok(_) => None,
        Err(e) => Some(e),
    }
}

fn load_active_skill_for_mode(
    state: &AppState,
    workspace_id: &str,
    skill_id: Option<&str>,
    mode: DataBoundaryMode,
) -> CommandResult<Option<Skill>> {
    let skill = load_skill_raw(state, workspace_id, skill_id)?;
    if let Some(skill) = &skill {
        if !skill.allows_mode(mode) {
            return err(format!(
                "skill `{}` does not allow data boundary `{}`",
                skill.id,
                mode.as_db_str()
            ));
        }
    }
    Ok(skill)
}

#[tauri::command]
pub async fn preview_data_boundary(
    state: State<'_, AppState>,
    request: CaseRunRequest,
) -> CommandResult<DataBoundaryPreview> {
    let api_key = match request.provider_id.as_str() {
        "ollama" | "apple-intelligence" | "anthropic-oauth" | "openai-oauth" => String::new(),
        other => secrets::load(other)
            .map_err(|e| e.to_string())?
            .unwrap_or_default(),
    };
    let provider = build_provider(
        &request.provider_id,
        &api_key,
        request.model.clone(),
        state.paths.config_dir(),
    )?;
    let mode = parse_data_boundary_mode(request.data_boundary_mode.as_deref());
    let skill_blocked_reason = skill_boundary_block_reason(
        &state,
        &request.workspace_id,
        request.active_skill_id.as_deref(),
        mode,
    );
    let purge_attachments_cfg = state
        .config
        .lock()
        .map_err(|_| "config poisoned")?
        .privacy
        .purge_attachments_with_raw_text;
    Ok(boundary_preview_for_request(
        &request,
        &provider,
        skill_blocked_reason,
        purge_attachments_cfg,
    ))
}

#[tauri::command]
pub async fn run_case(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: CaseRunRequest,
) -> CommandResult<CaseRunResponse> {
    run_case_impl(&app, &state, request, None).await
}

/// Drafted case ready for the LLM. The row is already persisted in
/// SQLite with status `Draft` and the attachments are on disk under the
/// canonical case directory — see [`stage_draft`].
struct StagedDraft {
    case: CaseRecord,
    attachments: Vec<CaseAttachment>,
}

/// Best-effort fallback when the frontend ships a case without an
/// explicit `patient_label`: pick the first attachment's filename stem,
/// or fall back to the first non-empty line of the clinical text trimmed
/// to ~60 chars. Returns an empty string if nothing usable is found —
/// the list renderer copes by falling back to the question or id.
fn derive_patient_label_from_request(request: &CaseRunRequest) -> String {
    if let Some(first) = request.attached_file_paths.first() {
        if let Some(name) = std::path::Path::new(first)
            .file_stem()
            .and_then(|s| s.to_str())
        {
            let stem = name.trim();
            if !stem.is_empty() {
                return stem.to_owned();
            }
        }
    }
    if let Some(line) = request.text.lines().find(|l| !l.trim().is_empty()) {
        let line = line.trim();
        if line.len() <= 60 {
            return line.to_owned();
        }
        return format!(
            "{}…",
            &line[..line.char_indices().nth(60).map_or(line.len(), |(i, _)| i)]
        );
    }
    String::new()
}

/// Generate a short patient-summary title using Apple Intelligence
/// (on-device, no network). Returns `None` when:
/// - the on-device model isn't available (non-Apple Silicon, off, not
///   downloaded, framework missing),
/// - the call times out (8 s budget),
/// - the model fails or returns something unusable.
///
/// On `None` the caller keeps whatever fallback label it already had
/// (filename stem). This is a polish, not a correctness path — never let
/// it block or fail the draft flow.
async fn try_apple_intelligence_label(
    masked_text: &str,
    attachments: &[CaseAttachment],
    language: &str,
) -> Option<String> {
    let provider = AppleIntelligenceProvider::new();
    if !matches!(
        provider.availability().await,
        AppleIntelligenceAvailability::Available
    ) {
        return None;
    }
    let system = format!(
        "You are a medical scribe assistant. Read the clinical material \
         and return ONE short title line (max 12 words) summarising the \
         patient, in {language}. Examples: \"Mujer 67, recto bajo T3N1 \
         alto riesgo\", \"Hombre 48, CCR derecho dMMR\". Return only the \
         title — no quotes, no prefix, no explanation."
    );
    let mut body = String::with_capacity(1024);
    let masked = masked_text.trim();
    if !masked.is_empty() {
        body.push_str("CLINICAL NARRATIVE:\n");
        body.push_str(&truncate_for_summary(masked, 800));
        body.push_str("\n\n");
    }
    if !attachments.is_empty() {
        body.push_str("ATTACHMENT EXCERPTS:\n");
        for (i, a) in attachments.iter().take(4).enumerate() {
            let snippet = a.extracted_text.trim();
            if snippet.is_empty() {
                continue;
            }
            body.push_str(&format!(
                "[{}] {}: {}\n",
                i + 1,
                a.original_filename,
                truncate_for_summary(snippet, 400)
            ));
        }
    }
    if body.trim().is_empty() {
        return None;
    }
    let req = CompletionRequest {
        model: String::new(),
        messages: vec![Message::system(system), Message::user(body)],
        max_output_tokens: Some(48),
        temperature: Some(0.2),
        json_schema: None,
        allow_web_search: false,
        images: Vec::new(),
    };
    let response =
        match tokio::time::timeout(std::time::Duration::from_secs(8), provider.complete(req)).await
        {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "apple-intelligence label generation failed");
                return None;
            }
            Err(_) => {
                tracing::warn!("apple-intelligence label generation timed out");
                return None;
            }
        };
    let raw = response.text.trim();
    if raw.is_empty() {
        return None;
    }
    // First non-empty line, stripped of common LLM decorations.
    let line = raw.lines().find(|l| !l.trim().is_empty())?.trim();
    let cleaned = line
        .trim_start_matches(['"', '\'', '*', '-', '#', ' '])
        .trim_end_matches(['"', '\'', '*', ' ', '.'])
        .trim();
    if cleaned.is_empty() {
        return None;
    }
    // Cap at 80 chars to avoid the model running away into a paragraph.
    let cap = cleaned
        .char_indices()
        .nth(80)
        .map_or(cleaned.len(), |(i, _)| i);
    Some(cleaned[..cap].to_owned())
}

fn truncate_for_summary(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_owned();
    }
    let cut = s.char_indices().nth(max_chars).map_or(s.len(), |(i, _)| i);
    format!("{}…", &s[..cut])
}

/// Fire-and-forget background task: try to upgrade the draft's
/// `patient_label` from the filename-stem fallback to a real
/// Apple-Intelligence-generated summary. Silently does nothing on hosts
/// where the on-device model isn't available — the existing label
/// already covers that case. When it succeeds, the row is updated AND a
/// `case:drafted` event is re-emitted so the existing frontend listener
/// refreshes the list (no new event type required).
fn spawn_label_upgrade(
    app: tauri::AppHandle,
    store: Arc<Mutex<CaseStore>>,
    workspace_id: String,
    case_id: String,
    masked_text: String,
    attachments: Vec<CaseAttachment>,
    language: String,
) {
    tokio::spawn(async move {
        let Some(label) = try_apple_intelligence_label(&masked_text, &attachments, &language).await
        else {
            return;
        };
        let updated = match store.lock() {
            Ok(g) => g.set_case_patient_label(&case_id, &label).is_ok(),
            Err(_) => false,
        };
        if !updated {
            return;
        }
        let _ = app.emit(
            "case:drafted",
            CaseDraftedDto {
                case_id,
                workspace_id,
            },
        );
    });
}

/// DTO emitted on `case:drafted` so the frontend can refresh the list
/// the moment a case (or a batch of cases) becomes persistable.
#[derive(Debug, Clone, Serialize)]
pub struct CaseDraftedDto {
    pub case_id: String,
    pub workspace_id: String,
}

/// Re-run Apple Intelligence title generation on an existing case.
///
/// Used by the CasesPage list to retry titles that stayed at the
/// filename-stem fallback because the first attempt timed out, failed,
/// or never ran (CLI-created cases). Silently no-ops when Apple
/// Intelligence isn't available or when no usable summary can be
/// produced — the existing label is preserved either way.
///
/// Returns the new label on success (so the caller can patch its local
/// state synchronously) and `None` when nothing was changed.
#[tauri::command]
pub async fn regenerate_case_label(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    workspace_id: String,
    case_id: String,
) -> CommandResult<Option<String>> {
    let workspace = workspace_manager(&state)
        .load(&workspace_id)
        .map_err(|e| e.to_string())?;
    let store = case_store_arc(&state, &workspace.id)?;
    // Load the case + attachments in a single locked block so we
    // release the mutex before the (slow) Apple Intelligence call.
    let (masked_text, attachments) = {
        let g = store.lock().map_err(|_| "store poisoned")?;
        let case = g
            .get_case(&case_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("case `{case_id}` not found"))?;
        let atts = g
            .list_attachments_for_case(&case_id)
            .map_err(|e| e.to_string())?;
        (case.masked_text, atts)
    };
    let language = workspace
        .language
        .clone()
        .unwrap_or_else(|| "es".to_owned());
    let Some(label) = try_apple_intelligence_label(&masked_text, &attachments, &language).await
    else {
        return Ok(None);
    };
    {
        let g = store.lock().map_err(|_| "store poisoned")?;
        g.set_case_patient_label(&case_id, &label)
            .map_err(|e| e.to_string())?;
    }
    // Re-emit case:drafted so the page-level listener refreshes and
    // picks up the new label without polling.
    let _ = app.emit(
        "case:drafted",
        CaseDraftedDto {
            case_id: case_id.clone(),
            workspace_id: workspace.id,
        },
    );
    Ok(Some(label))
}

/// Drafts-first staging shared by every case runner (quick, deliberative,
/// single, batch). Steps:
///
/// 1. Mint the canonical `case_id` up-front.
/// 2. De-identify the clinical text (when present) so `masked_text` is
///    the only copy that travels to disk / LLM.
/// 3. Ingest attachments **directly** under `cases/<case_id>/attachments/`
///    — no temp-dir shuffle. Attachment text is de-identified by
///    `ingest_case_attachments`.
/// 4. Insert the case row as `Draft` + the attachment rows in a single
///    locked block (no awaits while the mutex is held).
/// 5. Emit `case:drafted` so the UI can pop the row into the list
///    immediately, before the LLM call even starts.
///
/// On any failure the function returns early; nothing is persisted.
async fn stage_draft(
    app: &tauri::AppHandle,
    state: &AppState,
    workspace: &conclave_core::Workspace,
    store: &Arc<Mutex<CaseStore>>,
    request: &CaseRunRequest,
) -> CommandResult<StagedDraft> {
    let case_id = format!("case-{}", uuid::Uuid::new_v4());

    let deid = PipelineDeidentifier::new();
    let (masked_text, deident_pipeline_id, raw_text_sha256, raw_text_retention) =
        if request.text.trim().is_empty() {
            (
                String::new(),
                "noop".to_owned(),
                String::new(),
                RawTextRetention::Discarded,
            )
        } else {
            let r = deid.deidentify(&request.text).map_err(|e| e.to_string())?;
            (
                r.masked_text.clone(),
                r.pipeline_id.to_owned(),
                sha256_hex(request.text.as_bytes()),
                if request.retain_raw_text {
                    RawTextRetention::ExplicitRetained
                } else {
                    RawTextRetention::TemporaryDraft
                },
            )
        };

    let cases_root = state.paths.workspace_dir(&workspace.id).join("cases");
    let attachments = if request.attached_file_paths.is_empty() {
        Vec::new()
    } else {
        let paths: Vec<std::path::PathBuf> = request
            .attached_file_paths
            .iter()
            .map(std::path::PathBuf::from)
            .collect();
        let mut atts = ingest_case_attachments(paths, &case_id, &cases_root, &deid)
            .await
            .map_err(|e| e.to_string())?;
        for a in &mut atts {
            a.case_id.clone_from(&case_id);
        }
        atts
    };

    let now = chrono::Utc::now();
    let patient_label = if request.patient_label.trim().is_empty() {
        derive_patient_label_from_request(request)
    } else {
        request.patient_label.trim().to_owned()
    };
    let case = CaseRecord {
        id: case_id,
        created_at: now,
        case_date: now,
        workspace_id: workspace.id.clone(),
        question: request.question.clone(),
        original_text: request.text.clone(),
        masked_text,
        deident_pipeline_id,
        status: conclave_verdict::CaseStatus::Draft,
        patient_label,
        latest_error: None,
        raw_text_sha256,
        raw_text_retention,
    };

    {
        let g = store.lock().map_err(|_| "store poisoned")?;
        g.insert_case(&case).map_err(|e| e.to_string())?;
        for att in &attachments {
            if let Err(e) = g.insert_attachment(att) {
                tracing::warn!(error = ?e, "could not persist draft attachment row");
            }
        }
    }

    let _ = app.emit(
        "case:drafted",
        CaseDraftedDto {
            case_id: case.id.clone(),
            workspace_id: workspace.id.clone(),
        },
    );

    // Best-effort: try to upgrade the filename-stem fallback label into
    // a real patient summary using Apple Intelligence on capable hosts.
    // Runs in the background — the draft is already persisted and the
    // UI has already received it, so this is pure polish.
    spawn_label_upgrade(
        app.clone(),
        Arc::clone(store),
        workspace.id.clone(),
        case.id.clone(),
        case.masked_text.clone(),
        attachments.clone(),
        workspace
            .language
            .clone()
            .unwrap_or_else(|| "es".to_owned()),
    );

    Ok(StagedDraft { case, attachments })
}

/// Best-effort transition: when an LLM-side failure leaves a draft
/// orphaned, flip its status to `Failed` so the list reflects reality
/// instead of an eternally-loading row. The error message is logged at
/// ERROR level (so the dev server output surfaces what broke) AND
/// persisted on the case row via `set_case_error` so the detail view
/// can show it to the clinician.
fn mark_case_failed_best_effort(store: &Arc<Mutex<CaseStore>>, case_id: &str, err: &str) {
    tracing::error!(case_id, error = err, "case run failed — marking Failed");
    if let Ok(g) = store.lock() {
        if let Err(e) = g.mark_case_status(case_id, conclave_verdict::CaseStatus::Failed) {
            tracing::warn!(error = ?e, case_id, "could not mark draft as failed");
        }
        if let Err(e) = g.set_case_error(case_id, Some(err)) {
            tracing::warn!(error = ?e, case_id, "could not persist case error");
        }
    }
}

/// Free-function body of [`run_case`] so the batch runner can reuse it
/// without going through the Tauri IPC layer.
///
/// `batch_index` is `Some(idx)` when called from `run_batch_cases`.
/// Quick mode doesn't emit per-phase progress today so the parameter
/// is mostly reserved for future symmetry with
/// `run_case_deliberated_impl`.
pub(crate) async fn run_case_impl(
    app: &tauri::AppHandle,
    state: &AppState,
    request: CaseRunRequest,
    batch_index: Option<usize>,
) -> CommandResult<CaseRunResponse> {
    let _ = batch_index; // reserved for future use; silence unused warning
    let workspace = workspace_manager(state)
        .load(&request.workspace_id)
        .map_err(|e| e.to_string())?;
    // OAuth providers store credentials in a JSON file on disk, NOT in
    // the macOS keychain. Calling `secrets::load` on them returns None
    // AND triggers a Security framework call that deadlocks when fired
    // from N concurrent tokio tasks (the per-case workers in the batch
    // runner). Skip the keychain for every provider that doesn't use
    // it — mirrors the pattern at the Knowledge Q&A path above. CLI
    // providers (`claude-cli`, `codex-cli`) also live outside the
    // Conclave keychain — auth is the user's own CLI session.
    let api_key = match request.provider_id.as_str() {
        "ollama" | "apple-intelligence" | "anthropic-oauth" | "openai-oauth" | "claude-cli"
        | "codex-cli" => String::new(),
        other => secrets::load(other)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("no API key for `{other}`"))?,
    };
    let provider = build_provider(
        &request.provider_id,
        &api_key,
        request.model.clone(),
        state.paths.config_dir(),
    )?;
    ensure_general_scope(&provider)?;
    ensure_provider_ready(&provider).await?;
    let mode = parse_data_boundary_mode(request.data_boundary_mode.as_deref());
    let active_skill = load_active_skill_for_mode(
        state,
        &workspace.id,
        request.active_skill_id.as_deref(),
        mode,
    )?;
    let purge_attachments_cfg = state
        .config
        .lock()
        .map_err(|_| "config poisoned")?
        .privacy
        .purge_attachments_with_raw_text;
    let boundary = boundary_preview_for_request(&request, &provider, None, purge_attachments_cfg);
    enforce_data_boundary(&boundary)?;

    let store = case_store_arc(state, &workspace.id)?;
    let StagedDraft { case, attachments } =
        stage_draft(app, state, &workspace, &store, &request).await?;
    let external_evidence = if request.use_online_evidence {
        fetch_external_evidence_for_case(state, &case.masked_text, &case.question).await?
    } else {
        Vec::new()
    };

    // Register a per-case cancel flag so `cancel_case` can short-
    // circuit the run. Quick mode is a single LLM call so the check
    // happens before we go to the network rather than between phases.
    let case_cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let mut cancels = state.case_cancels.lock().await;
        cancels.insert(case.id.clone(), Arc::clone(&case_cancel));
    }
    if case_cancel.load(std::sync::atomic::Ordering::SeqCst) {
        let msg = conclave_verdict::deliberation::CANCELLED_MESSAGE.to_owned();
        mark_case_failed_best_effort(&store, &case.id, &msg);
        state.case_cancels.lock().await.remove(&case.id);
        return Err(msg);
    }

    let embedder = Arc::clone(&state.embedder);
    let repo = get_repo(state, &workspace.id).await?;
    let pipeline = VerdictPipeline::new(
        workspace.clone(),
        Box::new(PipelineDeidentifier::new()),
        embedder,
        repo,
        Arc::clone(&provider),
        Arc::clone(&store),
    );
    let mut options = VerdictOptions::default();
    let cfg = state.config.lock().map_err(|_| "config poisoned")?.clone();
    options.top_k = cfg.rag.top_k;
    if let Some(lang) = workspace.language.clone() {
        options.output_language = lang;
    }
    options.data_boundary_mode = boundary.mode;
    options.retain_raw_text = request.retain_raw_text;
    options.purge_attachment_files = cfg.privacy.purge_attachments_with_raw_text;
    options.external_evidence = external_evidence;
    if let Some(skill) = active_skill {
        options.active_skill_id = Some(skill.id);
        options.active_skill_instructions = Some(skill.body);
    }

    // `run_for_case` runs the pipeline against the already-persisted
    // draft and promotes it to `Completed` on success.
    let result = pipeline.run_for_case(&case, &attachments, &options).await;
    state.case_cancels.lock().await.remove(&case.id);
    match result {
        Ok(run) => Ok(CaseRunResponse {
            case: run.case,
            verdict_record: run.verdict_record,
            verdict: run.verdict,
            attachments,
            data_boundary: boundary,
        }),
        Err(e) => {
            let msg = e.to_string();
            mark_case_failed_best_effort(&store, &case.id, &msg);
            Err(msg)
        }
    }
}

#[tauri::command]
pub fn list_case_attachments(
    state: State<'_, AppState>,
    workspace_id: String,
    case_id: String,
) -> CommandResult<Vec<CaseAttachment>> {
    let store = case_store_arc(&state, &workspace_id)?;
    let store = store.lock().map_err(|_| "store poisoned")?;
    store
        .list_attachments_for_case(&case_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_deliberation_trace(
    state: State<'_, AppState>,
    workspace_id: String,
    verdict_id: String,
) -> CommandResult<Option<DeliberationTrace>> {
    let store = case_store_arc(&state, &workspace_id)?;
    let store = store.lock().map_err(|_| "store poisoned")?;
    store
        .get_deliberation_trace(&verdict_id)
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Draft cases — created from the classify-drop modal without running the
// committee. The clinician later opens a draft from the list, optionally
// adds clinical context in NewCase, and promotes it via `run_draft_case`.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateDraftCasesRequest {
    pub workspace_id: String,
    pub cases: Vec<crate::batch::BatchCaseInput>,
}

#[tauri::command]
pub async fn create_draft_cases(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: CreateDraftCasesRequest,
) -> CommandResult<Vec<CaseRecord>> {
    let workspace = workspace_manager(&state)
        .load(&request.workspace_id)
        .map_err(|e| e.to_string())?;
    let store = case_store_arc(&state, &workspace.id)?;
    let cases_root = state.paths.workspace_dir(&workspace.id).join("cases");
    let deid = PipelineDeidentifier::new();

    struct Staged {
        record: CaseRecord,
        attachments: Vec<CaseAttachment>,
    }
    let mut staged: Vec<Staged> = Vec::with_capacity(request.cases.len());

    for input in request.cases {
        let case_id = format!("case-{}", uuid::Uuid::new_v4());

        let (masked_text, deident_pipeline_id, raw_text_sha256, raw_text_retention) =
            if input.text.trim().is_empty() {
                (
                    String::new(),
                    "noop".to_owned(),
                    String::new(),
                    RawTextRetention::Discarded,
                )
            } else {
                let r = deid.deidentify(&input.text).map_err(|e| e.to_string())?;
                (
                    r.masked_text.clone(),
                    r.pipeline_id.to_owned(),
                    sha256_hex(input.text.as_bytes()),
                    RawTextRetention::TemporaryDraft,
                )
            };

        let attachments = if input.attached_file_paths.is_empty() {
            Vec::new()
        } else {
            let paths: Vec<std::path::PathBuf> = input
                .attached_file_paths
                .iter()
                .map(std::path::PathBuf::from)
                .collect();
            let mut atts = ingest_case_attachments(paths, &case_id, &cases_root, &deid)
                .await
                .map_err(|e| e.to_string())?;
            for a in &mut atts {
                a.case_id.clone_from(&case_id);
            }
            atts
        };

        let now = chrono::Utc::now();
        let record = CaseRecord {
            id: case_id,
            created_at: now,
            case_date: now,
            workspace_id: workspace.id.clone(),
            question: input.question.clone(),
            original_text: input.text.clone(),
            masked_text,
            deident_pipeline_id,
            status: conclave_verdict::CaseStatus::Draft,
            patient_label: input.patient_label.trim().to_owned(),
            latest_error: None,
            raw_text_sha256,
            raw_text_retention,
        };
        staged.push(Staged {
            record,
            attachments,
        });
    }

    let language = workspace
        .language
        .clone()
        .unwrap_or_else(|| "es".to_owned());

    let mut out = Vec::with_capacity(staged.len());
    let mut to_upgrade: Vec<(String, String, Vec<CaseAttachment>)> = Vec::new();
    {
        let store_guard = store.lock().map_err(|_| "store poisoned")?;
        for Staged {
            record,
            attachments,
        } in staged
        {
            store_guard
                .insert_case(&record)
                .map_err(|e| e.to_string())?;
            for att in &attachments {
                if let Err(e) = store_guard.insert_attachment(att) {
                    tracing::warn!(error = ?e, "could not persist draft attachment row");
                }
            }
            to_upgrade.push((
                record.id.clone(),
                record.masked_text.clone(),
                attachments.clone(),
            ));
            out.push(record);
        }
    }

    // Polish: try to upgrade each draft's filename-stem fallback into a
    // patient summary using Apple Intelligence. Background, best-effort.
    for (case_id, masked_text, attachments) in to_upgrade {
        spawn_label_upgrade(
            app.clone(),
            Arc::clone(&store),
            workspace.id.clone(),
            case_id,
            masked_text,
            attachments,
            language.clone(),
        );
    }

    Ok(out)
}

#[derive(Debug, Deserialize)]
pub struct RunDraftCaseRequest {
    pub workspace_id: String,
    pub case_id: String,
    pub provider_id: String,
    pub model: Option<String>,
    /// Optional override applied to the draft row before running.
    pub text: Option<String>,
    /// Optional override applied to the draft row before running.
    pub question: Option<String>,
    #[serde(default)]
    pub data_boundary_mode: Option<String>,
    #[serde(default)]
    pub allow_phi_payload: bool,
    #[serde(default)]
    pub retain_raw_text: bool,
    #[serde(default)]
    pub active_skill_id: Option<String>,
    #[serde(default)]
    pub use_online_evidence: bool,
}

#[tauri::command]
pub async fn run_draft_case(
    state: State<'_, AppState>,
    request: RunDraftCaseRequest,
) -> CommandResult<CaseRunResponse> {
    let workspace = workspace_manager(&state)
        .load(&request.workspace_id)
        .map_err(|e| e.to_string())?;
    let store = case_store_arc(&state, &workspace.id)?;

    // Load the draft + its attachments.
    let (mut draft, attachments) = {
        let g = store.lock().map_err(|_| "store poisoned")?;
        let case = g
            .get_case(&request.case_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("draft `{}` not found", request.case_id))?;
        if case.status != conclave_verdict::CaseStatus::Draft {
            return err(format!(
                "case `{}` is not a draft (status: {:?})",
                case.id, case.status
            ));
        }
        let atts = g
            .list_attachments_for_case(&request.case_id)
            .map_err(|e| e.to_string())?;
        (case, atts)
    };

    // Apply any clinical-context edits from NewCase before running.
    let deid = PipelineDeidentifier::new();
    if let Some(new_text) = request.text.clone() {
        let (masked, pipeline_id, hash, retention) = if new_text.trim().is_empty() {
            (
                String::new(),
                "noop".to_owned(),
                String::new(),
                RawTextRetention::Discarded,
            )
        } else {
            let r = deid.deidentify(&new_text).map_err(|e| e.to_string())?;
            (
                r.masked_text.clone(),
                r.pipeline_id.to_owned(),
                sha256_hex(new_text.as_bytes()),
                if request.retain_raw_text {
                    RawTextRetention::ExplicitRetained
                } else {
                    RawTextRetention::TemporaryDraft
                },
            )
        };
        draft.original_text = new_text;
        draft.masked_text = masked;
        draft.deident_pipeline_id = pipeline_id;
        draft.raw_text_sha256 = hash;
        draft.raw_text_retention = retention;
    }
    if let Some(new_question) = request.question.clone() {
        draft.question = new_question;
    }
    {
        let g = store.lock().map_err(|_| "store poisoned")?;
        g.update_case_draft_content(
            &draft.id,
            &draft.original_text,
            &draft.masked_text,
            &draft.deident_pipeline_id,
            &draft.question,
            &draft.raw_text_sha256,
            draft.raw_text_retention,
        )
        .map_err(|e| e.to_string())?;
    }

    // Provider + pipeline setup mirrors run_case_impl. OAuth providers
    // use disk-stored tokens, not the macOS keychain — skip secrets::load
    // for them. Same applies to the local CLI providers (`claude-cli`,
    // `codex-cli`): auth lives in the user's own CLI session.
    let api_key = match request.provider_id.as_str() {
        "ollama" | "apple-intelligence" | "anthropic-oauth" | "openai-oauth" | "claude-cli"
        | "codex-cli" => String::new(),
        other => secrets::load(other)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("no API key for `{other}`"))?,
    };
    let provider = build_provider(
        &request.provider_id,
        &api_key,
        request.model.clone(),
        state.paths.config_dir(),
    )?;
    ensure_general_scope(&provider)?;
    ensure_provider_ready(&provider).await?;
    let compat_request = CaseRunRequest {
        workspace_id: request.workspace_id.clone(),
        text: draft.original_text.clone(),
        question: draft.question.clone(),
        provider_id: request.provider_id.clone(),
        model: request.model.clone(),
        attached_file_paths: attachments.iter().map(|a| a.stored_path.clone()).collect(),
        patient_label: draft.patient_label.clone(),
        data_boundary_mode: request.data_boundary_mode.clone(),
        allow_phi_payload: request.allow_phi_payload,
        retain_raw_text: request.retain_raw_text,
        active_skill_id: request.active_skill_id.clone(),
        use_online_evidence: request.use_online_evidence,
    };
    let mode = parse_data_boundary_mode(request.data_boundary_mode.as_deref());
    let active_skill = load_active_skill_for_mode(
        &state,
        &workspace.id,
        request.active_skill_id.as_deref(),
        mode,
    )?;
    let purge_attachments_cfg = state
        .config
        .lock()
        .map_err(|_| "config poisoned")?
        .privacy
        .purge_attachments_with_raw_text;
    let boundary =
        boundary_preview_for_request(&compat_request, &provider, None, purge_attachments_cfg);
    enforce_data_boundary(&boundary)?;
    let external_evidence = if compat_request.use_online_evidence {
        fetch_external_evidence_for_case(&state, &draft.masked_text, &draft.question).await?
    } else {
        Vec::new()
    };
    let mut options = VerdictOptions::default();
    let cfg = state.config.lock().map_err(|_| "config poisoned")?.clone();
    options.top_k = cfg.rag.top_k;
    if let Some(lang) = workspace.language.clone() {
        options.output_language = lang;
    }
    options.data_boundary_mode = boundary.mode;
    options.retain_raw_text = request.retain_raw_text;
    options.purge_attachment_files = cfg.privacy.purge_attachments_with_raw_text;
    options.external_evidence = external_evidence;
    if let Some(skill) = active_skill {
        options.active_skill_id = Some(skill.id);
        options.active_skill_instructions = Some(skill.body);
    }

    let embedder = Arc::clone(&state.embedder);
    let repo = get_repo(&state, &workspace.id).await?;
    let pipeline = VerdictPipeline::new(
        workspace.clone(),
        Box::new(PipelineDeidentifier::new()),
        embedder,
        repo,
        provider,
        Arc::clone(&store),
    );
    match pipeline.run_for_case(&draft, &attachments, &options).await {
        Ok(run) => Ok(CaseRunResponse {
            case: run.case,
            verdict_record: run.verdict_record,
            verdict: run.verdict,
            attachments,
            data_boundary: boundary,
        }),
        Err(e) => {
            let msg = e.to_string();
            mark_case_failed_best_effort(&store, &draft.id, &msg);
            Err(msg)
        }
    }
}

// ---------------------------------------------------------------------------
// Deliberative mode — multi-pass committee with streaming progress events
// ---------------------------------------------------------------------------

/// Convert a stored attachment to an `ImageInput` for vision providers.
/// Returns `None` for non-image attachments, missing files, or unreadable
/// bytes (logged but never fatal — the caller continues with the rest).
async fn load_image_attachment(att: &CaseAttachment) -> Option<ImageInput> {
    use base64::Engine as _;
    if att.doc_type != "image" {
        return None;
    }
    let bytes = match tokio::fs::read(&att.stored_path).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, path = %att.stored_path, "could not read attachment for vision");
            return None;
        }
    };
    let base64_data = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let media_type = if att.mime.is_empty() {
        "image/png".to_owned()
    } else {
        att.mime.clone()
    };
    Some(ImageInput {
        media_type,
        base64_data,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeliberationEventDto {
    PhaseStarted {
        phase: String,
        case_id: String,
        batch_index: Option<usize>,
    },
    PhaseCompleted {
        phase: String,
        output: String,
        elapsed_ms: u64,
        case_id: String,
        batch_index: Option<usize>,
    },
    /// A phase hit a transient error and is being retried. `attempt`
    /// is the upcoming attempt number (e.g. 2 after the first failure).
    /// The UI uses this to swap the live phase badge from "Running" to
    /// "Retrying (N/2)" without persisting a Failed status.
    PhaseRetrying {
        phase: String,
        attempt: u8,
        reason: String,
        case_id: String,
        batch_index: Option<usize>,
    },
    PhaseFailed {
        phase: String,
        error: String,
        elapsed_ms: u64,
        case_id: String,
        batch_index: Option<usize>,
    },
    Done {
        verdict_json: String,
        case_id: String,
        batch_index: Option<usize>,
    },
}

#[tauri::command]
pub async fn run_case_deliberated(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: CaseRunRequest,
) -> CommandResult<CaseRunResponse> {
    run_case_deliberated_impl(&app, &state, request, None).await
}

/// Free-function body of [`run_case_deliberated`] for batch reuse.
///
/// `batch_index` is `Some(idx)` when this call is part of a
/// `run_batch_cases` run; the deliberation events are stamped with it
/// so the UI can route per-phase progress to the correct row inside
/// the batch table.
pub(crate) async fn run_case_deliberated_impl(
    app: &tauri::AppHandle,
    state: &AppState,
    request: CaseRunRequest,
    batch_index: Option<usize>,
) -> CommandResult<CaseRunResponse> {
    let workspace = workspace_manager(state)
        .load(&request.workspace_id)
        .map_err(|e| e.to_string())?;
    // OAuth providers use disk-stored tokens, not the macOS keychain.
    // See run_case_impl for the concurrent-deadlock rationale. CLI
    // providers (`claude-cli`, `codex-cli`) similarly have no Conclave
    // keychain entry — their auth lives in the user's own CLI session.
    let api_key = match request.provider_id.as_str() {
        "ollama" | "apple-intelligence" | "anthropic-oauth" | "openai-oauth" | "claude-cli"
        | "codex-cli" => String::new(),
        other => secrets::load(other)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("no API key for `{other}`"))?,
    };
    let provider = build_provider(
        &request.provider_id,
        &api_key,
        request.model.clone(),
        state.paths.config_dir(),
    )?;
    ensure_general_scope(&provider)?;
    ensure_provider_ready(&provider).await?;
    let mode = parse_data_boundary_mode(request.data_boundary_mode.as_deref());
    let active_skill = load_active_skill_for_mode(
        state,
        &workspace.id,
        request.active_skill_id.as_deref(),
        mode,
    )?;
    let purge_attachments_cfg = state
        .config
        .lock()
        .map_err(|_| "config poisoned")?
        .privacy
        .purge_attachments_with_raw_text;
    let boundary = boundary_preview_for_request(&request, &provider, None, purge_attachments_cfg);
    enforce_data_boundary(&boundary)?;

    let store = case_store_arc(state, &workspace.id)?;

    // Drafts-first: persist the case row + attachments before the (slow)
    // 4-pass deliberation begins. Frontend can render the draft right
    // away; we mark it `Completed`/`Failed` at the end.
    let StagedDraft { case, attachments } =
        stage_draft(app, state, &workspace, &store, &request).await?;
    let masked_text = case.masked_text.clone();
    let external_evidence = if request.use_online_evidence {
        fetch_external_evidence_for_case(state, &masked_text, &case.question).await?
    } else {
        Vec::new()
    };

    let cfg = state.config.lock().map_err(|_| "config poisoned")?.clone();
    let mut options = VerdictOptions::default();
    options.top_k = cfg.rag.top_k;
    if let Some(lang) = workspace.language.clone() {
        options.output_language = lang;
    }
    options.data_boundary_mode = boundary.mode;
    options.retain_raw_text = request.retain_raw_text;
    options.purge_attachment_files = cfg.privacy.purge_attachments_with_raw_text;
    if let Some(skill) = active_skill {
        options.active_skill_id = Some(skill.id);
        options.active_skill_instructions = Some(skill.body);
    }

    let embedder = Arc::clone(&state.embedder);
    let repo = get_repo(state, &workspace.id).await?;
    let masked_for_embed = masked_text.clone();
    let case_embedding = tokio::task::spawn_blocking(move || embedder.embed(&[masked_for_embed]))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?
        .into_iter()
        .next()
        .ok_or_else(|| "embedder returned no vectors".to_string())?;

    let mut evidence_chunks: Vec<DeliberationEvidence> = Vec::new();
    let mut evidence_refs: Vec<String> = Vec::new();
    if options.top_k > 0 {
        let hits = repo
            .search(&case_embedding, options.top_k)
            .await
            .map_err(|e| e.to_string())?;
        for (i, h) in hits.iter().enumerate() {
            let details = repo.show(&h.document_id).map_err(|e| e.to_string())?;
            let (title, doc_type) = match details {
                Some(d) => (
                    d.record.title,
                    format!("{:?}", d.record.doc_type).to_lowercase(),
                ),
                None => (h.document_id.clone(), "unknown".into()),
            };
            evidence_chunks.push(DeliberationEvidence {
                document_title: title,
                doc_type,
                snippet: truncate_str(&h.text, 1_200),
            });
            evidence_refs.push(format!("E{}", i + 1));
        }
    }

    let past_hits = {
        let g = store.lock().map_err(|_| "store poisoned")?;
        g.similar_past_cases(
            &case_embedding,
            options.past_cases_k,
            options.past_cases_min_similarity,
        )
        .map_err(|e| e.to_string())?
    };
    let past_cases: Vec<DeliberationPastCase> = past_hits
        .iter()
        .map(|h| DeliberationPastCase {
            feedback: h.feedback_kind.map_or("none", |k| k.as_db_str()).to_owned(),
            feedback_reason: h.feedback_reason.clone().unwrap_or_default(),
            case_summary: h.case_summary.clone(),
            verdict_summary: h.verdict_summary.clone(),
        })
        .collect();
    let past_refs: Vec<String> = (1..=past_cases.len()).map(|i| format!("P{i}")).collect();
    let attachment_refs: Vec<String> = (1..=attachments.len()).map(|i| format!("A{i}")).collect();
    let online_refs: Vec<String> = (1..=external_evidence.len())
        .map(|i| format!("X{i}"))
        .collect();

    let mut allowed: std::collections::HashSet<String> = evidence_refs.iter().cloned().collect();
    for r in &past_refs {
        allowed.insert(r.clone());
    }
    for r in &attachment_refs {
        allowed.insert(r.clone());
    }
    for r in &online_refs {
        allowed.insert(r.clone());
    }

    // 4) Build the image set ONCE, before the deliberation. Images live
    //    in memory only for the duration of this call.
    let mut images: Vec<ImageInput> = Vec::new();
    for att in &attachments {
        if let Some(img) = load_image_attachment(att).await {
            images.push(img);
        }
    }

    let specialty = workspace
        .specialty
        .clone()
        .unwrap_or_else(|| "medicina general".to_owned());

    let inputs = DeliberationInputs {
        specialty,
        output_language: options.output_language.clone(),
        rules_block: options.rules_block.clone(),
        masked_case_text: masked_text.clone(),
        user_question: request.question.clone(),
        active_skill_id: options.active_skill_id.clone(),
        active_skill_instructions: options.active_skill_instructions.clone(),
        evidence_chunks,
        external_evidence: external_evidence.clone(),
        past_cases,
        attachments: attachments.clone(),
        images,
    };

    // 5) Wire the streaming channel: spawn a task that forwards every
    //    event to the frontend as a Tauri `deliberation:progress`
    //    event. The forwarder stamps each event with the case id (so
    //    the UI can route per-phase progress to the correct row in
    //    batch mode) AND tracks phase start times so
    //    PhaseCompleted/PhaseFailed carry the wall-clock `elapsed_ms`.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<DeliberationEvent>();
    let app_for_forward = app.clone();
    let case_id_for_forward = case.id.clone();
    let batch_index_for_forward = batch_index;
    let forward_task = tokio::spawn(async move {
        let mut phase_starts: std::collections::HashMap<String, std::time::Instant> =
            std::collections::HashMap::new();
        while let Some(ev) = rx.recv().await {
            let dto = match ev {
                DeliberationEvent::PhaseStarted { phase } => {
                    let p = phase.as_str().to_owned();
                    phase_starts.insert(p.clone(), std::time::Instant::now());
                    DeliberationEventDto::PhaseStarted {
                        phase: p,
                        case_id: case_id_for_forward.clone(),
                        batch_index: batch_index_for_forward,
                    }
                }
                DeliberationEvent::PhaseCompleted { phase, output } => {
                    let p = phase.as_str().to_owned();
                    let elapsed_ms = phase_starts
                        .get(&p)
                        .map_or(0, |t| t.elapsed().as_millis() as u64);
                    DeliberationEventDto::PhaseCompleted {
                        phase: p,
                        output,
                        elapsed_ms,
                        case_id: case_id_for_forward.clone(),
                        batch_index: batch_index_for_forward,
                    }
                }
                DeliberationEvent::PhaseRetrying {
                    phase,
                    attempt,
                    reason,
                } => {
                    let p = phase.as_str().to_owned();
                    // Reset the phase start clock so the eventual
                    // PhaseCompleted/PhaseFailed elapsed_ms measures the
                    // *successful* attempt, not the original try.
                    phase_starts.insert(p.clone(), std::time::Instant::now());
                    DeliberationEventDto::PhaseRetrying {
                        phase: p,
                        attempt,
                        reason,
                        case_id: case_id_for_forward.clone(),
                        batch_index: batch_index_for_forward,
                    }
                }
                DeliberationEvent::PhaseFailed { phase, error } => {
                    let p = phase.as_str().to_owned();
                    let elapsed_ms = phase_starts
                        .get(&p)
                        .map_or(0, |t| t.elapsed().as_millis() as u64);
                    DeliberationEventDto::PhaseFailed {
                        phase: p,
                        error,
                        elapsed_ms,
                        case_id: case_id_for_forward.clone(),
                        batch_index: batch_index_for_forward,
                    }
                }
                DeliberationEvent::Done { verdict_json } => DeliberationEventDto::Done {
                    verdict_json,
                    case_id: case_id_for_forward.clone(),
                    batch_index: batch_index_for_forward,
                },
            };
            let _ = app_for_forward.emit("deliberation:progress", dto);
        }
    });

    // Register a per-case cancel flag so the `cancel_case` command can
    // stop this run at the next phase boundary. Removed in a
    // best-effort cleanup block after the run resolves.
    let case_cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let mut cancels = state.case_cancels.lock().await;
        cancels.insert(case.id.clone(), Arc::clone(&case_cancel));
    }
    let mut delib_opts = DeliberationOptions::default();
    delib_opts.cancel = Some(Arc::clone(&case_cancel));

    let deliberation_started = chrono::Utc::now();
    let deliberation_started_inst = std::time::Instant::now();
    let outcome =
        match run_deliberation(Arc::clone(&provider), inputs, allowed, delib_opts, tx).await {
            Ok(o) => o,
            Err(e) => {
                // Mark draft as failed so the UI stops showing "running…".
                let msg = e.to_string();
                mark_case_failed_best_effort(&store, &case.id, &msg);
                let _ = forward_task.await;
                state.case_cancels.lock().await.remove(&case.id);
                return Err(msg);
            }
        };
    let elapsed_ms = deliberation_started_inst.elapsed().as_millis() as u64;
    let _ = forward_task.await;
    state.case_cancels.lock().await.remove(&case.id);

    // The case row is already persisted as Draft. Build the verdict-side
    // records and promote to Completed in a single locked block below.
    let case_id = case.id.clone();
    let now = chrono::Utc::now();
    let verdict_record = VerdictRecord {
        id: format!("verdict-{}", uuid::Uuid::new_v4()),
        case_id: case.id.clone(),
        prompt_version: "verdict_v2_deliberated".to_owned(),
        provider_id: provider.id().to_owned(),
        model: outcome.model.clone(),
        latency_ms: elapsed_ms,
        input_tokens: outcome.trace.total_input_tokens,
        output_tokens: outcome.trace.total_output_tokens,
        output_json: serde_json::to_string(&outcome.verdict).unwrap_or_else(|_| "{}".into()),
        created_at: now,
    };
    let mut trace = outcome.trace.clone();
    trace.verdict_id = verdict_record.id.clone();
    trace.created_at = deliberation_started;
    trace.duration_ms = elapsed_ms;

    let retrieval = VerdictRetrievalTrace {
        verdict_id: verdict_record.id.clone(),
        evidence_refs,
        past_cases_refs: past_refs,
        online_evidence_refs: online_refs,
        attachment_refs,
    };

    // Persist case-memory entry so future cases can retrieve this one.
    let case_memory_summary = truncate_str(&outcome.verdict.case_summary, 1_200);
    let verdict_summary = truncate_str(
        &format!(
            "{} | {}",
            outcome.verdict.primary_recommendation.action, outcome.verdict.certainty_justification
        ),
        1_200,
    );

    // Promote draft → review-ready and persist the verdict-side records.
    // The case row and attachments already exist on disk + in SQLite.
    {
        let g = store.lock().map_err(|_| "store poisoned")?;
        g.insert_verdict(&verdict_record)
            .map_err(|e| e.to_string())?;
        g.insert_trace(&retrieval).map_err(|e| e.to_string())?;
        g.insert_deliberation_trace(&trace)
            .map_err(|e| e.to_string())?;
        g.insert_audit_run(&conclave_verdict::AuditRunRecord {
            id: format!("audit-{}", uuid::Uuid::new_v4()),
            case_id: case.id.clone(),
            verdict_id: Some(verdict_record.id.clone()),
            provider_id: provider.id().to_owned(),
            model: outcome.model.clone(),
            data_boundary_mode: boundary.mode,
            payload_mode: AuditPayloadMode::Fingerprint,
            active_skill_id: options.active_skill_id.clone(),
            started_at: deliberation_started,
            completed_at: Some(now),
            latency_ms: elapsed_ms,
            input_tokens: outcome.trace.total_input_tokens,
            output_tokens: outcome.trace.total_output_tokens,
            prompt_sha256: sha256_hex(format!("{masked_text}{retrieval:?}").as_bytes()),
            output_sha256: sha256_hex(verdict_record.output_json.as_bytes()),
            evidence_refs: retrieval.evidence_refs.clone(),
            past_cases_refs: retrieval.past_cases_refs.clone(),
            online_evidence_refs: retrieval.online_evidence_refs.clone(),
            attachment_refs: retrieval.attachment_refs.clone(),
            raw_text_retention: if request.retain_raw_text {
                case.raw_text_retention
            } else {
                RawTextRetention::Discarded
            },
            // Retained unless this run's policy actually purges files:
            // must stay the exact negation of the purge condition below.
            attachments_retained: request.retain_raw_text || !options.purge_attachment_files,
            status: "success".into(),
            error: None,
        })
        .map_err(|e| e.to_string())?;
        g.upsert_case_memory(
            &case.id,
            &case_embedding,
            &case_memory_summary,
            &verdict_summary,
        )
        .map_err(|e| e.to_string())?;
        g.mark_case_status(&case_id, conclave_verdict::CaseStatus::ReviewReady)
            .map_err(|e| e.to_string())?;
        if !request.retain_raw_text {
            g.purge_case_phi(&case_id).map_err(|e| e.to_string())?;
            if options.purge_attachment_files {
                g.purge_case_attachment_files(&case_id)
                    .map_err(|e| e.to_string())?;
            }
        }
    }

    // The promoted record carries the canonical `ReviewReady` status so
    // the frontend response matches what's now in SQLite.
    let mut promoted_case = case;
    promoted_case.status = conclave_verdict::CaseStatus::ReviewReady;
    if !request.retain_raw_text {
        promoted_case.original_text.clear();
        promoted_case.raw_text_retention = RawTextRetention::Discarded;
    }

    Ok(CaseRunResponse {
        case: promoted_case,
        verdict_record,
        verdict: outcome.verdict,
        attachments,
        data_boundary: boundary,
    })
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (count, c) in s.chars().enumerate() {
        if count >= max_chars {
            out.push('…');
            break;
        }
        out.push(c);
    }
    out
}

// ---------------------------------------------------------------------------
// Batch case ingestion — process several patients in one run
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn parse_batch_folder(
    folder_path: String,
    default_question: String,
) -> CommandResult<Vec<crate::batch::BatchCaseInput>> {
    crate::batch::parse_batch_folder(std::path::Path::new(&folder_path), &default_question)
}

#[tauri::command]
pub fn propose_case_grouping(
    paths: Vec<String>,
    default_question: String,
) -> CommandResult<Vec<crate::batch::BatchCaseInput>> {
    let pathbufs: Vec<std::path::PathBuf> =
        paths.into_iter().map(std::path::PathBuf::from).collect();
    Ok(crate::batch::propose_grouping_from_files(
        pathbufs,
        &default_question,
    ))
}

#[derive(Debug, Clone, Deserialize)]
pub struct BatchRunRequest {
    pub workspace_id: String,
    pub provider_id: String,
    pub model: Option<String>,
    /// When `true`, every case is processed via the deliberative pipeline.
    /// Defaults to `false` for the cheaper quick-mode pass.
    #[serde(default)]
    pub deliberative: bool,
    pub cases: Vec<crate::batch::BatchCaseInput>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BatchRunSummary {
    pub completed: usize,
    pub failed: usize,
    pub cancelled: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BatchEventDto {
    CaseQueued {
        index: usize,
        patient_label: String,
    },
    CaseStarted {
        index: usize,
        patient_label: String,
    },
    CaseCompleted {
        index: usize,
        patient_label: String,
        case_id: String,
    },
    CaseFailed {
        index: usize,
        patient_label: String,
        error: String,
    },
    CaseCancelled {
        index: usize,
        patient_label: String,
    },
    BatchDone {
        completed: usize,
        failed: usize,
        cancelled: usize,
    },
}

#[tauri::command]
pub async fn run_batch_cases(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: BatchRunRequest,
) -> CommandResult<BatchRunSummary> {
    // Fail-fast before announcing 100 cases as queued: build the
    // provider once and verify its scope. The per-case impls have the
    // same guard, but doing it up here turns a "all cases failed"
    // toast into a single clear error.
    // OAuth providers use disk-stored tokens, not the macOS keychain.
    // Skip secrets::load for them — see run_case_impl for the
    // concurrent-deadlock rationale.
    let probe_key = match request.provider_id.as_str() {
        "ollama" | "apple-intelligence" | "anthropic-oauth" | "openai-oauth" => String::new(),
        other => secrets::load(other)
            .map_err(|e| e.to_string())?
            .unwrap_or_default(),
    };
    let probe_provider = build_provider(
        &request.provider_id,
        &probe_key,
        request.model.clone(),
        state.paths.config_dir(),
    )?;
    ensure_general_scope(&probe_provider)?;
    ensure_provider_ready(&probe_provider).await?;
    drop(probe_provider);

    // Reset the cancellation flag for this batch.
    state
        .batch_cancel
        .store(false, std::sync::atomic::Ordering::SeqCst);

    // Announce every case as queued so the UI can render the full table
    // before work begins.
    for (i, c) in request.cases.iter().enumerate() {
        let _ = app.emit(
            "batch:progress",
            BatchEventDto::CaseQueued {
                index: i,
                patient_label: c.patient_label.clone(),
            },
        );
    }

    // Deliberative mode is heavier — keep the concurrency lower to avoid
    // hammering the provider rate limit. Quick mode can go a bit wider.
    let permits = if request.deliberative { 1 } else { 2 };

    use futures::stream::{self, StreamExt};
    use futures::FutureExt;

    // Reference borrows from the outer fn — captured by every async block
    // below. `buffer_unordered` keeps the futures alive on the same
    // function frame so non-`'static` borrows are fine.
    let state_ref: &AppState = &state;
    let workspace_id = request.workspace_id.clone();
    let provider_id = request.provider_id.clone();
    let model = request.model.clone();
    let deliberative = request.deliberative;
    let app_ref = app.clone();
    let cancel_ref = Arc::clone(&state.batch_cancel);

    let outcomes: Vec<BatchOutcome> = stream::iter(request.cases.into_iter().enumerate())
        .map(|(idx, case)| {
            let workspace_id = workspace_id.clone();
            let provider_id = provider_id.clone();
            let model = model.clone();
            let app_in_task = app_ref.clone();
            let cancel = Arc::clone(&cancel_ref);
            // A panic anywhere in the per-case pipeline (extractor,
            // de-identifier, provider) must not kill this whole command
            // future — that would freeze the batch banner forever with
            // no failed row and no `batch_done`. The per-case future is
            // wrapped in `catch_unwind` below so a panic degrades to a
            // visible CaseFailed and the batch moves on.
            let panic_label = case.patient_label.clone();
            let app_on_panic = app_ref.clone();
            let per_case = async move {
                if cancel.load(std::sync::atomic::Ordering::SeqCst) {
                    let _ = app_in_task.emit(
                        "batch:progress",
                        BatchEventDto::CaseCancelled {
                            index: idx,
                            patient_label: case.patient_label.clone(),
                        },
                    );
                    return BatchOutcome::Cancelled;
                }
                let _ = app_in_task.emit(
                    "batch:progress",
                    BatchEventDto::CaseStarted {
                        index: idx,
                        patient_label: case.patient_label.clone(),
                    },
                );
                let req = CaseRunRequest {
                    workspace_id,
                    text: case.text,
                    question: case.question,
                    provider_id,
                    model,
                    attached_file_paths: case.attached_file_paths,
                    patient_label: case.patient_label.clone(),
                    data_boundary_mode: None,
                    allow_phi_payload: false,
                    retain_raw_text: false,
                    active_skill_id: None,
                    use_online_evidence: false,
                };
                let result = if deliberative {
                    run_case_deliberated_impl(&app_in_task, state_ref, req, Some(idx)).await
                } else {
                    run_case_impl(&app_in_task, state_ref, req, Some(idx)).await
                };
                match result {
                    Ok(resp) => {
                        let _ = app_in_task.emit(
                            "batch:progress",
                            BatchEventDto::CaseCompleted {
                                index: idx,
                                patient_label: case.patient_label.clone(),
                                case_id: resp.case.id.clone(),
                            },
                        );
                        BatchOutcome::Completed
                    }
                    Err(e) => {
                        // Per-case cancel via `cancel_case`: surface as
                        // Cancelled rather than a noisy Failed row.
                        if e.contains(conclave_verdict::deliberation::CANCELLED_MESSAGE) {
                            let _ = app_in_task.emit(
                                "batch:progress",
                                BatchEventDto::CaseCancelled {
                                    index: idx,
                                    patient_label: case.patient_label.clone(),
                                },
                            );
                            return BatchOutcome::Cancelled;
                        }
                        // Fail-fast: a transport-level failure (Ollama
                        // offline, DNS issue, cloud provider 5xx) will
                        // hit every case the same way. Flip the cancel
                        // flag so the remaining queued cases short-
                        // circuit to Cancelled instead of burning 30 s
                        // of connect-timeout each.
                        if is_transport_failure(&e) {
                            cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                        let _ = app_in_task.emit(
                            "batch:progress",
                            BatchEventDto::CaseFailed {
                                index: idx,
                                patient_label: case.patient_label.clone(),
                                error: e,
                            },
                        );
                        BatchOutcome::Failed
                    }
                }
            };
            async move {
                match std::panic::AssertUnwindSafe(per_case).catch_unwind().await {
                    Ok(outcome) => outcome,
                    Err(panic) => {
                        let msg = panic
                            .downcast_ref::<&str>()
                            .map(|s| (*s).to_owned())
                            .or_else(|| panic.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "panicked".to_owned());
                        tracing::error!(index = idx, error = %msg, "batch case panicked");
                        let _ = app_on_panic.emit(
                            "batch:progress",
                            BatchEventDto::CaseFailed {
                                index: idx,
                                patient_label: panic_label,
                                error: format!("internal error: {msg}"),
                            },
                        );
                        BatchOutcome::Failed
                    }
                }
            }
        })
        .buffer_unordered(permits)
        .collect()
        .await;

    let mut summary = BatchRunSummary {
        completed: 0,
        failed: 0,
        cancelled: 0,
    };
    for o in outcomes {
        match o {
            BatchOutcome::Completed => summary.completed += 1,
            BatchOutcome::Failed => summary.failed += 1,
            BatchOutcome::Cancelled => summary.cancelled += 1,
        }
    }
    let _ = app.emit(
        "batch:progress",
        BatchEventDto::BatchDone {
            completed: summary.completed,
            failed: summary.failed,
            cancelled: summary.cancelled,
        },
    );
    Ok(summary)
}

#[tauri::command]
pub async fn batch_cancel(state: State<'_, AppState>) -> CommandResult<()> {
    state
        .batch_cancel
        .store(true, std::sync::atomic::Ordering::SeqCst);
    // The batch flag only short-circuits cases that haven't started yet
    // (see the `cancel.load()` guard in `run_batch_cases`). Flip every
    // registered per-case flag too so the in-flight deliberative case
    // aborts at its next phase boundary — same mechanism as `cancel_case`.
    let cancels = state.case_cancels.lock().await;
    for flag in cancels.values() {
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    Ok(())
}

/// Cancel a single in-flight case. Looks up the per-case `AtomicBool`
/// registered by `run_case_*_impl` and flips it to `true`. The
/// deliberation pipeline checks the flag at every phase boundary and
/// returns early with [`CANCELLED_MESSAGE`]; the batch worker then
/// emits `BatchEventDto::CaseCancelled` for that index.
///
/// No-op (returns Ok) when the id isn't currently running — keeps the
/// UI happy even if the user clicks cancel after the case completes.
#[tauri::command]
pub async fn cancel_case(state: State<'_, AppState>, case_id: String) -> CommandResult<()> {
    let cancels = state.case_cancels.lock().await;
    if let Some(flag) = cancels.get(&case_id) {
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    Ok(())
}

/// Reset a previously-Failed case row back to Draft so the user can
/// retry it without losing its attachments. Used by the inline Retry
/// affordance on a failed row.
#[tauri::command]
pub fn reset_case_to_draft(
    state: State<'_, AppState>,
    workspace_id: String,
    case_id: String,
) -> CommandResult<CaseRecord> {
    let store = case_store_arc(&state, &workspace_id)?;
    let store = store.lock().map_err(|_| "store poisoned")?;
    store
        .mark_case_status(&case_id, conclave_verdict::CaseStatus::Draft)
        .map_err(|e| e.to_string())?;
    store
        .set_case_error(&case_id, None)
        .map_err(|e| e.to_string())?;
    let case = store
        .get_case(&case_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("case `{case_id}` not found"))?;
    Ok(case)
}

enum BatchOutcome {
    Completed,
    Failed,
    Cancelled,
}

#[tauri::command]
pub fn list_cases(
    state: State<'_, AppState>,
    workspace_id: String,
    limit: usize,
) -> CommandResult<Vec<CaseRecord>> {
    let store = case_store_arc(&state, &workspace_id)?;
    let store = store.lock().map_err(|_| "store poisoned")?;
    store.list_cases(limit).map_err(|e| e.to_string())
}

#[derive(Debug, Serialize)]
pub struct CaseDetail {
    pub case: CaseRecord,
    pub verdict_record: Option<VerdictRecord>,
    pub verdict: Option<Verdict>,
    pub attachments: Vec<CaseAttachment>,
    pub audit: Option<conclave_verdict::AuditRunRecord>,
    pub review: Option<ReviewMetadataRecord>,
}

#[tauri::command]
pub fn show_case(
    state: State<'_, AppState>,
    workspace_id: String,
    id: String,
) -> CommandResult<Option<CaseDetail>> {
    let store = case_store_arc(&state, &workspace_id)?;
    let store = store.lock().map_err(|_| "store poisoned")?;
    let Some(case) = store.get_case(&id).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let verdict_record = store.latest_verdict(&id).map_err(|e| e.to_string())?;
    let verdict = verdict_record
        .as_ref()
        .and_then(|v| serde_json::from_str::<Verdict>(&v.output_json).ok());
    let audit = store
        .latest_audit_for_case(&id)
        .map_err(|e| e.to_string())?;
    let review = store.get_review_metadata(&id).map_err(|e| e.to_string())?;
    let attachments = store
        .list_attachments_for_case(&id)
        .map_err(|e| e.to_string())?;
    Ok(Some(CaseDetail {
        case,
        verdict_record,
        verdict,
        attachments,
        audit,
        review,
    }))
}

#[derive(Debug, Deserialize)]
pub struct FeedbackRequest {
    pub workspace_id: String,
    pub case_id: String,
    pub kind: String,
    pub reason: Option<String>,
    pub reviewer_name: Option<String>,
    pub reviewer_role: Option<String>,
    pub final_verdict_json: Option<String>,
}

#[tauri::command]
pub fn submit_feedback(state: State<'_, AppState>, request: FeedbackRequest) -> CommandResult<()> {
    let kind = match request.kind.as_str() {
        "accept" => FeedbackKind::Accept,
        "modify" => FeedbackKind::Modify,
        "reject" => FeedbackKind::Reject,
        other => return err(format!("unknown feedback kind `{other}`")),
    };
    let store = case_store_arc(&state, &request.workspace_id)?;
    let store = store.lock().map_err(|_| "store poisoned")?;
    let verdict = store
        .latest_verdict(&request.case_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no verdict found for case `{}`", request.case_id))?;
    store
        .upsert_feedback(&FeedbackRecord {
            case_id: request.case_id.clone(),
            kind,
            reason: request.reason.clone(),
            modified_verdict_json: request.final_verdict_json.clone(),
            created_at: chrono::Utc::now(),
        })
        .map_err(|e| e.to_string())?;
    let decision = match kind {
        FeedbackKind::Accept => ReviewDecision::Accept,
        FeedbackKind::Modify => ReviewDecision::Modify,
        FeedbackKind::Reject => ReviewDecision::Reject,
    };
    let diff_summary = request
        .final_verdict_json
        .as_ref()
        .and_then(|final_json| summarize_json_diff(&verdict.output_json, final_json));
    store
        .finalize_review(&ReviewMetadataRecord {
            case_id: request.case_id,
            verdict_id: verdict.id,
            decision,
            reviewer_name: request.reviewer_name,
            reviewer_role: request.reviewer_role,
            note: request.reason,
            final_verdict_json: request.final_verdict_json,
            diff_summary,
            reviewed_at: chrono::Utc::now(),
        })
        .map_err(|e| e.to_string())
}

fn summarize_json_diff(original_json: &str, final_json: &str) -> Option<String> {
    if original_json.trim() == final_json.trim() {
        return None;
    }
    let Ok(original) = serde_json::from_str::<serde_json::Value>(original_json) else {
        return Some("Final verdict text differs from generated draft".to_owned());
    };
    let Ok(final_value) = serde_json::from_str::<serde_json::Value>(final_json) else {
        return Some("Final verdict text differs from generated draft".to_owned());
    };
    if original == final_value {
        return None;
    }
    let mut changed = Vec::new();
    if let (Some(o), Some(f)) = (original.as_object(), final_value.as_object()) {
        for key in f.keys() {
            if o.get(key) != f.get(key) {
                changed.push(key.clone());
            }
        }
        for key in o.keys() {
            if !f.contains_key(key) {
                changed.push(key.clone());
            }
        }
        changed.sort();
        changed.dedup();
    }
    if changed.is_empty() {
        Some("Final verdict JSON differs from generated draft".to_owned())
    } else {
        Some(format!("Changed fields: {}", changed.join(", ")))
    }
}

#[tauri::command]
pub fn purge_case_phi(
    state: State<'_, AppState>,
    workspace_id: String,
    case_id: String,
) -> CommandResult<CaseRecord> {
    let store = case_store_arc(&state, &workspace_id)?;
    let store = store.lock().map_err(|_| "store poisoned")?;
    store.purge_case_phi(&case_id).map_err(|e| e.to_string())?;
    store
        .get_case(&case_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("case `{case_id}` not found"))
}

#[tauri::command]
pub fn purge_case_attachments(
    state: State<'_, AppState>,
    workspace_id: String,
    case_id: String,
) -> CommandResult<usize> {
    let store = case_store_arc(&state, &workspace_id)?;
    let store = store.lock().map_err(|_| "store poisoned")?;
    store
        .purge_case_attachment_files(&case_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn audit_status(
    state: State<'_, AppState>,
    workspace_id: String,
) -> CommandResult<conclave_verdict::AuditStatus> {
    let store = case_store_arc(&state, &workspace_id)?;
    let store = store.lock().map_err(|_| "store poisoned")?;
    store.audit_status().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_audit_runs(
    state: State<'_, AppState>,
    workspace_id: String,
    limit: usize,
) -> CommandResult<Vec<conclave_verdict::AuditRunRecord>> {
    let store = case_store_arc(&state, &workspace_id)?;
    let store = store.lock().map_err(|_| "store poisoned")?;
    store.list_audit_runs(limit).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn export_audit_runs(
    state: State<'_, AppState>,
    workspace_id: String,
) -> CommandResult<Vec<conclave_verdict::AuditRunRecord>> {
    let store = case_store_arc(&state, &workspace_id)?;
    let store = store.lock().map_err(|_| "store poisoned")?;
    store.list_audit_runs(usize::MAX).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_skills(
    state: State<'_, AppState>,
    workspace_id: String,
) -> CommandResult<Vec<conclave_verdict::Skill>> {
    let user_skills = state.paths.config_dir().join("skills");
    let workspace_skills = state.paths.workspace_dir(&workspace_id).join("skills");
    conclave_verdict::load_skills(Some(&user_skills), Some(&workspace_skills))
        .map_err(|e| e.to_string())
}

#[derive(Debug, Deserialize)]
pub struct UpdateCaseDateRequest {
    pub workspace_id: String,
    pub case_ids: Vec<String>,
    /// RFC3339 timestamp. The UI typically sends a `datetime-local` value
    /// converted to ISO 8601 with the user's local offset.
    pub new_date: String,
}

#[tauri::command]
pub fn update_case_date(
    state: State<'_, AppState>,
    request: UpdateCaseDateRequest,
) -> CommandResult<()> {
    if request.case_ids.is_empty() {
        return Ok(());
    }
    let new_date = chrono::DateTime::parse_from_rfc3339(&request.new_date)
        .map_err(|e| format!("invalid date `{}`: {e}", request.new_date))?
        .with_timezone(&chrono::Utc);
    let store = case_store_arc(&state, &request.workspace_id)?;
    let mut store = store.lock().map_err(|_| "store poisoned")?;
    store
        .update_case_date(&request.case_ids, new_date)
        .map_err(|e| e.to_string())
}

#[derive(Debug, Deserialize)]
pub struct DeleteCasesRequest {
    pub workspace_id: String,
    pub case_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct DeleteCasesResponse {
    /// Number of `cases` rows actually removed. Ids that didn't exist
    /// in the DB are silently skipped.
    pub deleted: usize,
}

/// Delete one or many cases. SQLite cascades clean every child row
/// (verdicts, retrieval traces, feedback, case_memory, attachments,
/// deliberation traces). After the DB transaction commits we
/// best-effort remove each case's on-disk attachments directory at
/// `<workspace>/cases/<case_id>/` — failures there are logged but
/// do NOT roll back the DB delete, since the user has already seen
/// the row disappear.
#[tauri::command]
pub fn delete_cases(
    state: State<'_, AppState>,
    request: DeleteCasesRequest,
) -> CommandResult<DeleteCasesResponse> {
    if request.case_ids.is_empty() {
        return Ok(DeleteCasesResponse { deleted: 0 });
    }
    let store = case_store_arc(&state, &request.workspace_id)?;
    let cases_root = state
        .paths
        .workspace_dir(&request.workspace_id)
        .join("cases");

    let deleted = {
        let mut store = store.lock().map_err(|_| "store poisoned")?;
        store
            .delete_cases(&request.case_ids)
            .map_err(|e| e.to_string())?
    };

    // Best-effort cleanup of the on-disk case directories. Missing
    // directories are not an error (Draft cases without attachments
    // never create one).
    for id in &request.case_ids {
        let dir = cases_root.join(id);
        if !dir.exists() {
            continue;
        }
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            tracing::warn!(
                error = %e,
                path = %dir.display(),
                "could not remove case attachments dir after delete",
            );
        }
    }

    Ok(DeleteCasesResponse { deleted })
}

// ---------------------------------------------------------------------------
// CLI provider diagnostics
//
// These commands back the in-Settings "CLI setup" panel. The panel
// renders when the user clicks a `claude-cli` or `codex-cli` tile whose
// status is anything other than `Ready` — i.e. either the binary is
// missing from `$PATH` (`NotInstalled`) or the user hasn't run
// `claude auth login` / `codex login` yet (`LoginRequired`).
//
// The panel needs two things the regular `list_providers` payload
// doesn't carry:
//
// 1. Diagnostics — the resolved binary path (when present), the actual
//    process `$PATH` Conclave was launched with, and the documented
//    install URL / login command. We surface the PATH so a user with an
//    unusual install location can immediately see why detection failed
//    instead of guessing.
// 2. A re-detection trigger — `RwLock`-backed cache invalidation that
//    forces the next `is_installed` call to re-walk `$PATH`. The user
//    clicks "Volver a detectar" after installing the CLI in another
//    window without restarting Conclave.
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct CliDiagnostics {
    /// Resolved absolute path to the binary on `$PATH`, or `None` when
    /// `which::which` couldn't find it.
    pub binary_path: Option<String>,
    /// The full process `$PATH` as Conclave currently sees it. Used by
    /// the UI's collapsible debug block so the user can confirm whether
    /// their custom install dir is on the path the bundled app sees.
    pub path_var: String,
    /// Vendor-documented install URL (opened in the system browser).
    pub install_url: String,
    /// The exact terminal command the user must run to authenticate.
    /// Surfaced verbatim with a copy-to-clipboard affordance.
    pub login_command: String,
    /// Same probe result that `list_providers` reports — the panel uses
    /// it to decide which variant copy to show (not_installed,
    /// login_required, expired, ready).
    pub status: ProviderStatus,
    /// Raw probe payload (command run, exit code, duration, stderr
    /// excerpt, fallback used). Rendered under the "Diagnóstico
    /// técnico" disclosure so the next "no detecta" report ships with
    /// enough detail to diagnose in one screenshot.
    pub probe: ProbeDetails,
    /// `true` when the user clicked "Marcar como conectado" for this
    /// provider — the manual override safety net. Mirrors
    /// `Config.providers.cli_local_overrides[id]`.
    pub user_marked_ready: bool,
}

/// Return diagnostic info for one of the CLI providers. Rejects any
/// non-CLI id so the frontend can't accidentally fan this command out.
#[tauri::command]
pub async fn cli_diagnostics(
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<CliDiagnostics> {
    let user_marked_ready = state
        .config
        .lock()
        .map_err(|_| "config poisoned")?
        .providers
        .cli_local_overrides
        .get(&id)
        .copied()
        .unwrap_or(false);

    let (binary_path, probe, install_url, login_command) = match id.as_str() {
        "claude-cli" => {
            let installed = ClaudeCliProvider::is_installed();
            let probe = if installed {
                ClaudeCliProvider::probe_login_detailed().await
            } else {
                ProbeDetails::unresolved_binary()
            };
            (
                ClaudeCliProvider::binary_path().map(|p| p.display().to_string()),
                probe,
                "https://docs.claude.com/en/docs/agents/claude-code/overview".to_owned(),
                "claude auth login".to_owned(),
            )
        }
        "codex-cli" => {
            let installed = CodexCliProvider::is_installed();
            let probe = if installed {
                CodexCliProvider::probe_login_detailed().await
            } else {
                ProbeDetails::unresolved_binary()
            };
            (
                CodexCliProvider::binary_path().map(|p| p.display().to_string()),
                probe,
                "https://github.com/openai/codex".to_owned(),
                "codex login".to_owned(),
            )
        }
        _ => return Err(format!("cli_diagnostics: unsupported provider id `{id}`")),
    };

    let installed = binary_path.is_some();
    let logged_in = probe.logged_in || (installed && user_marked_ready);
    let status = if !installed {
        ProviderStatus::NotInstalled
    } else if !logged_in {
        ProviderStatus::LoginRequired
    } else {
        ProviderStatus::Ready
    };

    Ok(CliDiagnostics {
        binary_path,
        path_var: std::env::var("PATH").unwrap_or_default(),
        install_url,
        login_command,
        status,
        probe,
        user_marked_ready,
    })
}

/// Invalidate the in-process `which::which` cache for both CLI
/// providers so the next probe re-walks `$PATH`. Called from the
/// "Volver a detectar" button in the CLI setup panel.
#[tauri::command]
pub async fn redetect_cli_binaries() -> CommandResult<()> {
    ClaudeCliProvider::refresh_binary_cache();
    CodexCliProvider::refresh_binary_cache();
    Ok(())
}

/// Toggle the manual "I'm logged in, trust me" override for one CLI
/// provider id. Persisted to `conclave.toml` so the user only needs to
/// declare it once. `value: false` (or absent) removes the entry — we
/// don't keep negative entries because their meaning is identical to
/// "no override at all".
///
/// When enabling the override we also clear any stale entry in
/// `disabled_provider_ids` for the same id — the two flags would
/// otherwise fight each other (`disabled` wins, demoting the override
/// back to `NotConfigured`). Disabling the override is left alone so
/// the user can still "Disconnect" + un-override + reconnect later.
///
/// Only accepts the two CLI provider ids; everything else 400s so the
/// frontend can't accidentally write garbage keys.
#[tauri::command]
pub async fn set_cli_login_override(
    state: State<'_, AppState>,
    id: String,
    value: bool,
) -> CommandResult<()> {
    if !matches!(id.as_str(), "claude-cli" | "codex-cli") {
        return Err(format!(
            "set_cli_login_override: unsupported provider id `{id}`"
        ));
    }
    let mut cfg = state.config.lock().map_err(|_| "config poisoned")?.clone();
    if value {
        cfg.providers.cli_local_overrides.insert(id.clone(), true);
        // Enabling the override implies "I want this provider usable."
        // Clear any stale disable entry so list_providers doesn't
        // demote it back to NotConfigured.
        cfg.providers.disabled_provider_ids.retain(|d| d != &id);
    } else {
        cfg.providers.cli_local_overrides.remove(&id);
    }
    cfg.save(state.paths.config_file())
        .map_err(|e| e.to_string())?;
    *state.config.lock().map_err(|_| "config poisoned")? = cfg;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_transport_failure_catches_ollama_offline_message() {
        // Verbatim string we ship from `ensure_provider_ready` when the
        // pre-flight ping fails.
        let msg = "Ollama is not responding at http://localhost:11434. \
                   Start the server with `ollama serve` or pick another \
                   provider from the selector.";
        assert!(is_transport_failure(msg));
    }

    #[test]
    fn is_transport_failure_catches_provider_error_chain() {
        // Real-world chain we observed before the fix:
        //   ProviderError::Network → Error::Provider → CommandResult string.
        let msg = "provider error: draft LLM call failed: network error: \
                   error sending request for url (http://localhost:11434/api/chat)";
        assert!(is_transport_failure(msg));
    }

    #[test]
    fn is_transport_failure_catches_unavailable_variant() {
        let msg = "provider error: provider unavailable: model not downloaded";
        assert!(is_transport_failure(msg));
    }

    #[test]
    fn is_transport_failure_catches_revoked_claude_cli_session() {
        // Shape produced by the claude-cli provider when the stored
        // OAuth token is rejected with a 401 — one dead token fails
        // every case, so the batch must short-circuit.
        let msg = "provider error: deliberation phase briefing failed: \
                   provider unavailable: Claude CLI is signed out or its \
                   session was revoked (the API rejected the stored token). \
                   Run `claude auth login` in a terminal, then retry.";
        assert!(is_transport_failure(msg));
    }

    #[test]
    fn is_transport_failure_catches_auth_failed_variant() {
        // `ProviderError::Auth` Display ("authentication failed") from
        // API-key providers — a revoked key is structural for the batch.
        let msg = "provider error: deliberation phase drafting failed: \
                   authentication failed";
        assert!(is_transport_failure(msg));
    }

    #[test]
    fn is_transport_failure_ignores_validation_errors() {
        // Validation / schema errors are NOT transport-level — they hit
        // one case but the next case might be fine, so the batch must
        // keep running. Guard against false positives here.
        assert!(!is_transport_failure(
            "provider error: verdict validation failed: invalid ref `Z9`"
        ));
        assert!(!is_transport_failure(
            "provider error: bad request: model `gpt-9` not found"
        ));
        assert!(!is_transport_failure(
            "rag error: case store mutex poisoned"
        ));
    }

    /// `ensure_provider_ready` pings Ollama when the provider id matches.
    /// Pointing at a closed port surfaces a clear, actionable error
    /// instead of letting the per-case run discover it 11 times.
    #[tokio::test]
    async fn ensure_provider_ready_errors_when_ollama_unreachable() {
        let p: Arc<dyn LlmProvider> =
            Arc::new(OllamaProvider::new().with_base_url("http://127.0.0.1:65500"));
        let r = ensure_provider_ready(&p).await;
        assert!(r.is_err(), "expected Err for unreachable Ollama");
        let msg = r.unwrap_err();
        assert!(
            msg.contains("Ollama is not responding"),
            "unexpected error text: {msg}"
        );
    }

    /// Cloud providers are not pinged — the trip into the network for a
    /// pre-flight would be redundant with the first per-case call.
    #[tokio::test]
    async fn ensure_provider_ready_passes_for_cloud_providers() {
        let p: Arc<dyn LlmProvider> = Arc::new(AnthropicProvider::new("sk-fake-key-not-used"));
        let r = ensure_provider_ready(&p).await;
        assert!(
            r.is_ok(),
            "cloud providers should not require ping; got {r:?}"
        );
    }
}
