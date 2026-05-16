//! Tauri commands — thin wrappers over the Rust core crates. Every error
//! is mapped to a String so the frontend can render it directly.

use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::State;

use conclave_core::{Workspace, WorkspaceManager};
use conclave_deident::{Deidentifier, PipelineDeidentifier};
use conclave_providers::{
    secrets, AnthropicProvider, CompletionRequest, LlmProvider, Message, OllamaProvider,
    OpenAiProvider, OpenRouterProvider, KNOWN_PROVIDERS,
};
use conclave_rag::{
    ChunkParams, DocumentRecord, DocumentRepository, Embedder, FastEmbedEmbedder, IngestionEvent,
    IngestionPipeline, RepositoryLayout,
};
use conclave_verdict::{
    persistence::{FeedbackKind, FeedbackRecord},
    CaseRecord, CaseStore, Verdict, VerdictOptions, VerdictPipeline, VerdictRecord,
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
    pub disclaimer: String,
}

#[tauri::command]
pub fn onboarding_status(state: State<'_, AppState>) -> CommandResult<OnboardingStatus> {
    let path = state.paths.config_dir().join(DISCLAIMER_MARKER);
    Ok(OnboardingStatus {
        accepted: path.exists(),
        disclaimer: conclave_core::MEDICAL_DISCLAIMER.to_owned(),
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
pub fn delete_workspace(state: State<'_, AppState>, id_or_name: String) -> CommandResult<()> {
    workspace_manager(&state)
        .delete(&id_or_name)
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Documents (knowledge base)
// ---------------------------------------------------------------------------

async fn open_repo(
    state: &AppState,
    workspace_id: &str,
    embedder: &Arc<dyn Embedder>,
) -> Result<Arc<DocumentRepository>, String> {
    let dir = state.paths.workspace_dir(workspace_id);
    let layout = RepositoryLayout::new(dir);
    let repo = DocumentRepository::open(layout, embedder.dim())
        .await
        .map_err(|e| e.to_string())?;
    Ok(Arc::new(repo))
}

#[tauri::command]
pub async fn list_documents(
    state: State<'_, AppState>,
    workspace_id: String,
) -> CommandResult<Vec<DocumentRecord>> {
    let embedder: Arc<dyn Embedder> = Arc::new(FastEmbedEmbedder::new());
    let repo = open_repo(&state, &workspace_id, &embedder).await?;
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
    let embedder: Arc<dyn Embedder> = Arc::new(FastEmbedEmbedder::new());
    let repo = open_repo(&state, &workspace_id, &embedder).await?;
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
    let embedder: Arc<dyn Embedder> = Arc::new(FastEmbedEmbedder::new());
    let repo = open_repo(&state, &workspace_id, &embedder).await?;
    repo.remove(&id).await.map_err(|e| e.to_string())
}

#[derive(Debug, Serialize)]
pub struct IngestSummary {
    pub ingested: usize,
    pub skipped: usize,
    pub failed: usize,
    pub messages: Vec<String>,
}

#[tauri::command]
pub async fn ingest_path(
    state: State<'_, AppState>,
    workspace_id: String,
    path: String,
) -> CommandResult<IngestSummary> {
    let embedder: Arc<dyn Embedder> = Arc::new(FastEmbedEmbedder::new());
    let repo = open_repo(&state, &workspace_id, &embedder).await?;
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

#[derive(Debug, Serialize)]
pub struct SearchHit {
    pub chunk_id: String,
    pub document_id: String,
    pub text: String,
    pub distance: f32,
}

#[tauri::command]
pub async fn search_workspace(
    state: State<'_, AppState>,
    workspace_id: String,
    query: String,
    k: usize,
) -> CommandResult<Vec<SearchHit>> {
    let embedder: Arc<dyn Embedder> = Arc::new(FastEmbedEmbedder::new());
    let repo = open_repo(&state, &workspace_id, &embedder).await?;
    let embedder_for_query = Arc::clone(&embedder);
    let q = query.clone();
    let vectors = tokio::task::spawn_blocking(move || embedder_for_query.embed(&[q]))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let Some(vec) = vectors.into_iter().next() else {
        return ok(Vec::new());
    };
    let hits = repo.search(&vec, k).await.map_err(|e| e.to_string())?;
    Ok(hits
        .into_iter()
        .map(|h| SearchHit {
            chunk_id: h.chunk_id,
            document_id: h.document_id,
            text: h.text,
            distance: h.distance,
        })
        .collect())
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
}

#[tauri::command]
pub async fn list_providers() -> CommandResult<Vec<ProviderInfo>> {
    let mut out = Vec::new();
    for id in KNOWN_PROVIDERS {
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
        });
    }
    Ok(out)
}

#[tauri::command]
pub async fn set_provider_key(id: String, api_key: String) -> CommandResult<()> {
    if id == "ollama" {
        return ok(());
    }
    if api_key.trim().is_empty() {
        return err("API key cannot be empty");
    }
    secrets::store(&id, &api_key).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn test_provider(id: String, prompt: Option<String>) -> CommandResult<String> {
    let api_key = if id == "ollama" {
        String::new()
    } else {
        match secrets::load(&id).map_err(|e| e.to_string())? {
            Some(k) => k,
            None => return err(format!("no API key for {id}")),
        }
    };
    let provider = build_provider(&id, &api_key, None)?;
    let prompt = prompt.unwrap_or_else(|| "Reply with one word: hello.".into());
    let resp = provider
        .complete(CompletionRequest {
            model: String::new(),
            messages: vec![Message::user(prompt)],
            max_output_tokens: Some(50),
            temperature: Some(0.0),
            json_schema: None,
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

fn build_provider(
    id: &str,
    api_key: &str,
    model: Option<String>,
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
        other => return Err(format!("unknown provider `{other}`")),
    })
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
}

#[derive(Debug, Deserialize)]
pub struct CaseRunRequest {
    pub workspace_id: String,
    pub text: String,
    pub question: String,
    pub provider_id: String,
    pub model: Option<String>,
}

#[tauri::command]
pub async fn run_case(
    state: State<'_, AppState>,
    request: CaseRunRequest,
) -> CommandResult<CaseRunResponse> {
    let workspace = workspace_manager(&state)
        .load(&request.workspace_id)
        .map_err(|e| e.to_string())?;
    let api_key = if request.provider_id == "ollama" {
        String::new()
    } else {
        match secrets::load(&request.provider_id).map_err(|e| e.to_string())? {
            Some(k) => k,
            None => return err(format!("no API key for `{}`", request.provider_id)),
        }
    };
    let provider = build_provider(&request.provider_id, &api_key, request.model.clone())?;

    let embedder: Arc<dyn Embedder> = Arc::new(FastEmbedEmbedder::new());
    let repo = open_repo(&state, &workspace.id, &embedder).await?;
    let store = case_store_arc(&state, &workspace.id)?;
    let pipeline = VerdictPipeline::new(
        workspace.clone(),
        Box::new(PipelineDeidentifier::new()),
        embedder,
        repo,
        provider,
        store,
    );
    let mut options = VerdictOptions::default();
    let cfg = state.config.lock().map_err(|_| "config poisoned")?.clone();
    options.top_k = cfg.rag.top_k;
    if let Some(lang) = workspace.language.clone() {
        options.output_language = lang;
    }
    let run = pipeline
        .run(&request.text, &request.question, &options)
        .await
        .map_err(|e| e.to_string())?;
    Ok(CaseRunResponse {
        case: run.case,
        verdict_record: run.verdict_record,
        verdict: run.verdict,
    })
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
