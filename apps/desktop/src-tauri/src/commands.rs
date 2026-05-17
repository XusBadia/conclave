//! Tauri commands — thin wrappers over the Rust core crates. Every error
//! is mapped to a String so the frontend can render it directly.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

use conclave_core::{Workspace, WorkspaceManager};
use conclave_deident::{Deidentifier, PipelineDeidentifier};
use conclave_providers::{
    open_in_browser, persist_tokens, secrets, AnthropicLoginFlow, AnthropicOAuthProvider,
    AnthropicProvider, AppleIntelligenceAvailability, AppleIntelligenceProvider, CompletionRequest,
    ImageInput, LlmProvider, Message, OllamaProvider, OpenAILoginFlow, OpenAIOAuthProvider,
    OpenAiProvider, OpenRouterProvider, ProviderScope, APPLE_INTELLIGENCE_MODEL_LABEL,
    KNOWN_PROVIDERS, OAUTH_PROVIDERS,
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
    CaseAttachment, CaseRecord, CaseStore, QaPipeline, Verdict, VerdictOptions, VerdictPipeline,
    VerdictRecord,
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
    let api_key = match request.provider_id.as_str() {
        "ollama" | "apple-intelligence" | "anthropic-oauth" | "openai-oauth" => String::new(),
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

#[derive(Debug, Serialize)]
pub struct ProviderInfo {
    pub id: String,
    pub configured: bool,
    pub available: bool,
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
        // surfaced (always present, configured == reachable).
        configured: available,
        available,
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

#[tauri::command]
pub async fn list_providers(state: State<'_, AppState>) -> CommandResult<Vec<ProviderInfo>> {
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
        let (available, default_model, requires_net) = match *id {
            "ollama" => {
                let p = OllamaProvider::new();
                (p.ping().await, "llama3.1:8b".into(), false)
            }
            "anthropic" => (configured, "claude-sonnet-4-6-20250929".into(), true),
            "openai" => (configured, "gpt-5".into(), true),
            "openrouter" => (configured, "set per call".into(), true),
            _ => (false, "—".into(), false),
        };
        out.push(ProviderInfo {
            id: (*id).to_owned(),
            configured,
            available,
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
        let (configured, available, hint) = if conclave_path.exists() {
            let hint = match *id {
                "anthropic-oauth" => AnthropicOAuthProvider::from_conclave_tokens(&conclave_path)
                    .ok()
                    .and_then(|p| p.subscription_type()),
                "openai-oauth" => OpenAIOAuthProvider::from_conclave_tokens(&conclave_path)
                    .ok()
                    .and_then(|p| p.account_label()),
                _ => None,
            };
            (true, true, hint)
        } else {
            (false, false, Some("sign in to start".into()))
        };
        let default_model = match *id {
            "anthropic-oauth" => "claude-sonnet-4-6-20250929".into(),
            "openai-oauth" => "gpt-5.5".into(),
            _ => "—".into(),
        };
        out.push(ProviderInfo {
            id: (*id).to_owned(),
            configured,
            available,
            default_model,
            requires_network: true,
            auth: "oauth".into(),
            kind: "oauth".into(),
            hint,
        });
    }
    Ok(out)
}

#[tauri::command]
pub async fn set_provider_key(id: String, api_key: String) -> CommandResult<()> {
    if matches!(id.as_str(), "ollama" | "apple-intelligence") {
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
        "ollama" | "apple-intelligence" | "anthropic-oauth" | "openai-oauth" => String::new(),
        _ => match secrets::load(&id).map_err(|e| e.to_string())? {
            Some(k) => k,
            None => return err(format!("no API key for {id}")),
        },
    };
    let provider = build_provider(&id, &api_key, None, state.paths.config_dir())?;
    let prompt = prompt.unwrap_or_else(|| "Reply with one word: hello.".into());
    let resp = provider
        .complete(CompletionRequest {
            model: String::new(),
            messages: vec![Message::user(prompt)],
            max_output_tokens: Some(50),
            temperature: Some(0.0),
            json_schema: None,
            allow_web_search: false,
            images: Vec::new(),
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "{}\n\n— {} ({}+{} tokens)",
        resp.text, resp.model, resp.usage.input_tokens, resp.usage.output_tokens
    ))
}

#[tauri::command]
pub fn remove_provider_key(id: String) -> CommandResult<()> {
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
pub fn oauth_logout(state: State<'_, AppState>, id: String) -> CommandResult<()> {
    let path = state
        .paths
        .config_dir()
        .join("oauth")
        .join(format!("{id}.json"));
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
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
}

#[tauri::command]
pub async fn run_case(
    state: State<'_, AppState>,
    request: CaseRunRequest,
) -> CommandResult<CaseRunResponse> {
    run_case_impl(&state, request).await
}

/// Free-function body of [`run_case`] so the batch runner can reuse it
/// without going through the Tauri IPC layer. Kept in sync with
/// [`run_case`] line-for-line — if you change one, change the other.
pub(crate) async fn run_case_impl(
    state: &AppState,
    request: CaseRunRequest,
) -> CommandResult<CaseRunResponse> {
    let workspace = workspace_manager(state)
        .load(&request.workspace_id)
        .map_err(|e| e.to_string())?;
    let api_key = if matches!(
        request.provider_id.as_str(),
        "ollama" | "apple-intelligence"
    ) {
        String::new()
    } else {
        match secrets::load(&request.provider_id).map_err(|e| e.to_string())? {
            Some(k) => k,
            None => return err(format!("no API key for `{}`", request.provider_id)),
        }
    };
    let provider = build_provider(
        &request.provider_id,
        &api_key,
        request.model.clone(),
        state.paths.config_dir(),
    )?;
    ensure_general_scope(&provider)?;

    let embedder = Arc::clone(&state.embedder);
    let repo = get_repo(state, &workspace.id).await?;
    let store = case_store_arc(state, &workspace.id)?;
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

    // Stage attachments BEFORE the LLM call so we can pass them into the
    // pipeline and pre-allocate a case id that owns the on-disk folder.
    let case_id_for_attachments = format!("case-{}", uuid::Uuid::new_v4());
    let attachments = if request.attached_file_paths.is_empty() {
        Vec::new()
    } else {
        let cases_root = state.paths.workspace_dir(&workspace.id).join("cases");
        let deid = PipelineDeidentifier::new();
        let paths: Vec<std::path::PathBuf> = request
            .attached_file_paths
            .iter()
            .map(std::path::PathBuf::from)
            .collect();
        ingest_case_attachments(paths, &case_id_for_attachments, &cases_root, &deid)
            .await
            .map_err(|e| e.to_string())?
    };

    let run = pipeline
        .run(&request.text, &request.question, &attachments, &options)
        .await
        .map_err(|e| e.to_string())?;

    // Re-stamp attachments with the canonical case id minted by the
    // pipeline and move them to the final per-case directory. We do all
    // file IO first (await-friendly) and only acquire the DB lock once,
    // synchronously, to avoid holding a non-Send guard across awaits.
    let mut moved_attachments = Vec::with_capacity(attachments.len());
    let workspace_cases = state.paths.workspace_dir(&workspace.id).join("cases");
    if !attachments.is_empty() {
        let final_dir = workspace_cases.join(&run.case.id).join("attachments");
        if let Err(e) = tokio::fs::create_dir_all(&final_dir).await {
            tracing::warn!(error = %e, "could not create attachments dir for case");
        }
        for mut att in attachments {
            att.case_id = run.case.id.clone();
            let src = std::path::PathBuf::from(&att.stored_path);
            if let Some(filename) = src.file_name() {
                let dst = final_dir.join(filename);
                if src != dst {
                    if let Err(e) = tokio::fs::rename(&src, &dst).await {
                        tracing::debug!(error = %e, src = %src.display(), "could not move attachment to final dir");
                    } else {
                        att.stored_path = dst.to_string_lossy().into_owned();
                    }
                }
            }
            moved_attachments.push(att);
        }
        // Best-effort: remove the now-empty temp directory minted during
        // ingest. Ignore errors — leftover empty dirs are harmless.
        let temp_dir = workspace_cases.join(&case_id_for_attachments);
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }

    // Persist DB rows synchronously, in its own scoped block, so the
    // MutexGuard is dropped before any subsequent code that might await.
    let persisted_attachments = {
        let store_guard = store.lock().map_err(|_| "store poisoned")?;
        for att in &moved_attachments {
            if let Err(e) = store_guard.insert_attachment(att) {
                tracing::warn!(error = ?e, "could not persist attachment row");
            }
        }
        moved_attachments
    };

    Ok(CaseRunResponse {
        case: run.case,
        verdict_record: run.verdict_record,
        verdict: run.verdict,
        attachments: persisted_attachments,
    })
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

        let (masked_text, deident_pipeline_id) = if input.text.trim().is_empty() {
            (String::new(), "noop".to_owned())
        } else {
            let r = deid.deidentify(&input.text).map_err(|e| e.to_string())?;
            (r.masked_text.clone(), r.pipeline_id.to_owned())
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
        };
        staged.push(Staged {
            record,
            attachments,
        });
    }

    let store_guard = store.lock().map_err(|_| "store poisoned")?;
    let mut out = Vec::with_capacity(staged.len());
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
        out.push(record);
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
        let (masked, pipeline_id) = if new_text.trim().is_empty() {
            (String::new(), "noop".to_owned())
        } else {
            let r = deid.deidentify(&new_text).map_err(|e| e.to_string())?;
            (r.masked_text.clone(), r.pipeline_id.to_owned())
        };
        draft.original_text = new_text;
        draft.masked_text = masked;
        draft.deident_pipeline_id = pipeline_id;
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
        )
        .map_err(|e| e.to_string())?;
    }

    // Provider + pipeline setup mirrors run_case_impl.
    let api_key = if request.provider_id == "ollama" {
        String::new()
    } else {
        match secrets::load(&request.provider_id).map_err(|e| e.to_string())? {
            Some(k) => k,
            None => return err(format!("no API key for `{}`", request.provider_id)),
        }
    };
    let provider = build_provider(
        &request.provider_id,
        &api_key,
        request.model.clone(),
        state.paths.config_dir(),
    )?;
    let mut options = VerdictOptions::default();
    let cfg = state.config.lock().map_err(|_| "config poisoned")?.clone();
    options.top_k = cfg.rag.top_k;
    if let Some(lang) = workspace.language.clone() {
        options.output_language = lang;
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
    let run = pipeline
        .run_for_case(&draft, &attachments, &options)
        .await
        .map_err(|e| e.to_string())?;

    Ok(CaseRunResponse {
        case: run.case,
        verdict_record: run.verdict_record,
        verdict: run.verdict,
        attachments,
    })
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
    PhaseStarted { phase: String },
    PhaseCompleted { phase: String, output: String },
    PhaseFailed { phase: String, error: String },
    Done { verdict_json: String },
}

impl DeliberationEventDto {
    fn from_event(ev: &DeliberationEvent) -> Self {
        match ev {
            DeliberationEvent::PhaseStarted { phase } => Self::PhaseStarted {
                phase: phase.as_str().to_owned(),
            },
            DeliberationEvent::PhaseCompleted { phase, output } => Self::PhaseCompleted {
                phase: phase.as_str().to_owned(),
                output: output.clone(),
            },
            DeliberationEvent::PhaseFailed { phase, error } => Self::PhaseFailed {
                phase: phase.as_str().to_owned(),
                error: error.clone(),
            },
            DeliberationEvent::Done { verdict_json } => Self::Done {
                verdict_json: verdict_json.clone(),
            },
        }
    }
}

#[tauri::command]
pub async fn run_case_deliberated(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: CaseRunRequest,
) -> CommandResult<CaseRunResponse> {
    run_case_deliberated_impl(&app, &state, request).await
}

/// Free-function body of [`run_case_deliberated`] for batch reuse.
pub(crate) async fn run_case_deliberated_impl(
    app: &tauri::AppHandle,
    state: &AppState,
    request: CaseRunRequest,
) -> CommandResult<CaseRunResponse> {
    let workspace = workspace_manager(state)
        .load(&request.workspace_id)
        .map_err(|e| e.to_string())?;
    let api_key = if matches!(
        request.provider_id.as_str(),
        "ollama" | "apple-intelligence"
    ) {
        String::new()
    } else {
        match secrets::load(&request.provider_id).map_err(|e| e.to_string())? {
            Some(k) => k,
            None => return err(format!("no API key for `{}`", request.provider_id)),
        }
    };
    let provider = build_provider(
        &request.provider_id,
        &api_key,
        request.model.clone(),
        state.paths.config_dir(),
    )?;
    ensure_general_scope(&provider)?;

    let store = case_store_arc(state, &workspace.id)?;

    // 1) De-identify the case text up-front (the deliberation works on the
    //    masked text only — same invariant as the quick pipeline).
    let deid = PipelineDeidentifier::new();
    let deident_result = deid.deidentify(&request.text).map_err(|e| e.to_string())?;
    let masked_text = deident_result.masked_text.clone();
    let deident_pipeline_id = deident_result.pipeline_id.to_owned();

    // 2) Ingest attachments (same flow as run_case — local-only).
    let case_id_for_attachments = format!("case-{}", uuid::Uuid::new_v4());
    let cases_root = state.paths.workspace_dir(&workspace.id).join("cases");
    let attachments = if request.attached_file_paths.is_empty() {
        Vec::new()
    } else {
        let paths: Vec<std::path::PathBuf> = request
            .attached_file_paths
            .iter()
            .map(std::path::PathBuf::from)
            .collect();
        ingest_case_attachments(paths, &case_id_for_attachments, &cases_root, &deid)
            .await
            .map_err(|e| e.to_string())?
    };

    // 3) Retrieve workspace knowledge-base evidence + similar past cases
    //    using the same plumbing as the quick pipeline.
    let cfg = state.config.lock().map_err(|_| "config poisoned")?.clone();
    let mut options = VerdictOptions::default();
    options.top_k = cfg.rag.top_k;
    if let Some(lang) = workspace.language.clone() {
        options.output_language = lang;
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

    let mut allowed: std::collections::HashSet<String> = evidence_refs.iter().cloned().collect();
    for r in &past_refs {
        allowed.insert(r.clone());
    }
    for r in &attachment_refs {
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
        evidence_chunks,
        past_cases,
        attachments: attachments.clone(),
        images,
    };

    // 5) Wire the streaming channel: spawn a task that forwards every
    //    event to the frontend as a Tauri `deliberation:progress` event.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<DeliberationEvent>();
    let app_for_forward = app.clone();
    let forward_task = tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            let _ = app_for_forward.emit(
                "deliberation:progress",
                DeliberationEventDto::from_event(&ev),
            );
        }
    });

    let deliberation_started = chrono::Utc::now();
    let deliberation_started_inst = std::time::Instant::now();
    let outcome = run_deliberation(
        Arc::clone(&provider),
        inputs,
        allowed,
        DeliberationOptions::default(),
        tx,
    )
    .await
    .map_err(|e| e.to_string())?;
    let elapsed_ms = deliberation_started_inst.elapsed().as_millis() as u64;
    let _ = forward_task.await;

    // 6) Persist case + verdict + trace. Same shape as the quick path so
    //    list_cases / show_case continue to work uniformly.
    let case_id = format!("case-{}", uuid::Uuid::new_v4());
    let now = chrono::Utc::now();
    let case = CaseRecord {
        id: case_id.clone(),
        created_at: now,
        case_date: now,
        workspace_id: workspace.id.clone(),
        question: request.question.clone(),
        original_text: request.text.clone(),
        masked_text: masked_text.clone(),
        deident_pipeline_id,
        status: conclave_verdict::CaseStatus::Completed,
    };
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
        online_evidence_refs: Vec::new(),
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

    // Move attachments to the final case-id directory, mirroring run_case.
    let mut moved_attachments = Vec::with_capacity(attachments.len());
    if !attachments.is_empty() {
        let workspace_cases = state.paths.workspace_dir(&workspace.id).join("cases");
        let final_dir = workspace_cases.join(&case_id).join("attachments");
        if let Err(e) = tokio::fs::create_dir_all(&final_dir).await {
            tracing::warn!(error = %e, "could not create attachments dir");
        }
        for mut att in attachments {
            att.case_id = case_id.clone();
            let src = std::path::PathBuf::from(&att.stored_path);
            if let Some(filename) = src.file_name() {
                let dst = final_dir.join(filename);
                if src != dst {
                    if let Err(e) = tokio::fs::rename(&src, &dst).await {
                        tracing::debug!(error = %e, "could not move attachment to final dir");
                    } else {
                        att.stored_path = dst.to_string_lossy().into_owned();
                    }
                }
            }
            moved_attachments.push(att);
        }
        let temp_dir = workspace_cases.join(&case_id_for_attachments);
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }

    let persisted_attachments = {
        let g = store.lock().map_err(|_| "store poisoned")?;
        g.insert_case(&case).map_err(|e| e.to_string())?;
        g.insert_verdict(&verdict_record)
            .map_err(|e| e.to_string())?;
        g.insert_trace(&retrieval).map_err(|e| e.to_string())?;
        g.insert_deliberation_trace(&trace)
            .map_err(|e| e.to_string())?;
        g.upsert_case_memory(
            &case.id,
            &case_embedding,
            &case_memory_summary,
            &verdict_summary,
        )
        .map_err(|e| e.to_string())?;
        for att in &moved_attachments {
            if let Err(e) = g.insert_attachment(att) {
                tracing::warn!(error = ?e, "could not persist attachment row");
            }
        }
        moved_attachments
    };

    Ok(CaseRunResponse {
        case,
        verdict_record,
        verdict: outcome.verdict,
        attachments: persisted_attachments,
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
    let probe_key = if matches!(
        request.provider_id.as_str(),
        "ollama" | "apple-intelligence"
    ) {
        String::new()
    } else {
        secrets::load(&request.provider_id)
            .map_err(|e| e.to_string())?
            .unwrap_or_default()
    };
    let probe_provider = build_provider(
        &request.provider_id,
        &probe_key,
        request.model.clone(),
        state.paths.config_dir(),
    )?;
    ensure_general_scope(&probe_provider)?;
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
            async move {
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
                };
                let result = if deliberative {
                    run_case_deliberated_impl(&app_in_task, state_ref, req).await
                } else {
                    run_case_impl(state_ref, req).await
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
pub fn batch_cancel(state: State<'_, AppState>) -> CommandResult<()> {
    state
        .batch_cancel
        .store(true, std::sync::atomic::Ordering::SeqCst);
    Ok(())
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
    Ok(Some(CaseDetail {
        case,
        verdict_record,
        verdict,
    }))
}

#[derive(Debug, Deserialize)]
pub struct FeedbackRequest {
    pub workspace_id: String,
    pub case_id: String,
    pub kind: String,
    pub reason: Option<String>,
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
    store
        .upsert_feedback(&FeedbackRecord {
            case_id: request.case_id,
            kind,
            reason: request.reason,
            modified_verdict_json: None,
            created_at: chrono::Utc::now(),
        })
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
