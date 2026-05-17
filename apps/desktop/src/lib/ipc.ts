import { invoke } from "@tauri-apps/api/core";

import { isOccupyingSlot } from "./providers";

// ---------------------------------------------------------------------------
// Types — mirror the Tauri command return values in
// `apps/desktop/src-tauri/src/commands.rs`.
// ---------------------------------------------------------------------------

export interface Workspace {
  id: string;
  name: string;
  specialty?: string | null;
  language?: string | null;
  created_at: string;
}

export interface OnboardingStatus {
  accepted: boolean;
  /** English disclaimer copy (legacy field — equal to `disclaimer_en`). */
  disclaimer: string;
  disclaimer_en: string;
  disclaimer_es: string;
}

export interface DocumentRecord {
  id: string;
  source_path: string;
  copied_path: string;
  title: string;
  doc_type: "pdf" | "docx" | "txt" | "md" | "html";
  sha256: string;
  ingested_at: string;
  page_count: number;
  status: "ready" | "needs_ocr" | "failed";
}

export interface DocumentDetail {
  record: DocumentRecord;
  chunk_count: number;
  sample_text: string | null;
}

export interface IngestSummary {
  ingested: number;
  skipped: number;
  failed: number;
  messages: string[];
}

export interface QaSource {
  index: number;
  document_id: string;
  document_title: string;
  chunk_id: string;
  snippet: string;
}

export interface WebSource {
  url: string;
  title: string;
  snippet: string;
}

export interface AskDocumentsResponse {
  answer: string;
  sources: QaSource[];
  web_sources: WebSource[];
  model: string;
  input_tokens: number;
  output_tokens: number;
}

export type IngestStage = "extracting" | "chunking" | "embedding" | "storing";

export type IngestProgressEvent =
  | { kind: "starting"; path: string }
  | { kind: "progress"; path: string; stage: IngestStage; percent: number }
  | { kind: "ingested"; path: string; doc_id: string }
  | { kind: "skipped"; path: string; reason: string }
  | { kind: "failed"; path: string; error: string };

export interface DeidentResponse {
  masked_text: string;
  span_count: number;
  strict_clean: boolean;
}

export interface ProviderInfo {
  id: string;
  configured: boolean;
  available: boolean;
  default_model: string;
  requires_network: boolean;
  auth: "api-key" | "local" | "oauth";
  kind: "standard" | "oauth";
  hint: string | null;
}

export interface Verdict {
  case_summary: string;
  key_clinical_data: { label: string; value: string }[];
  applied_evidence: { ref: string; claim: string }[];
  primary_recommendation: { action: string; rationale: string };
  alternatives: { action: string; when_to_consider: string }[];
  certainty_level: "high" | "medium" | "low";
  certainty_justification: string;
  red_flags: string[];
  follow_up_triggers: string[];
  disclaimer: string;
}

export interface CaseRecord {
  id: string;
  created_at: string;
  workspace_id: string;
  question: string;
  original_text: string;
  masked_text: string;
  deident_pipeline_id: string;
  status: "completed" | "failed";
}

export interface VerdictRecord {
  id: string;
  case_id: string;
  prompt_version: string;
  provider_id: string;
  model: string;
  latency_ms: number;
  input_tokens: number;
  output_tokens: number;
  output_json: string;
  created_at: string;
}

export interface CaseRunResponse {
  case: CaseRecord;
  verdict_record: VerdictRecord;
  verdict: Verdict;
}

export interface CaseDetail {
  case: CaseRecord;
  verdict_record: VerdictRecord | null;
  verdict: Verdict | null;
}

// ---------------------------------------------------------------------------
// Typed wrappers
// ---------------------------------------------------------------------------

export const ipc = {
  // Onboarding
  onboardingStatus: () => invoke<OnboardingStatus>("onboarding_status"),
  acceptDisclaimer: () => invoke<void>("accept_disclaimer"),

  // Workspaces
  listWorkspaces: () => invoke<Workspace[]>("list_workspaces"),
  createWorkspace: (name: string, specialty?: string, language?: string) =>
    invoke<Workspace>("create_workspace", { name, specialty, language }),
  switchWorkspace: (idOrName: string) =>
    invoke<Workspace>("switch_workspace", { idOrName }),
  activeWorkspace: () => invoke<Workspace | null>("active_workspace"),
  deleteWorkspace: (idOrName: string) =>
    invoke<void>("delete_workspace", { idOrName }),

  // Documents
  listDocuments: (workspaceId: string) =>
    invoke<DocumentRecord[]>("list_documents", { workspaceId }),
  showDocument: (workspaceId: string, id: string) =>
    invoke<DocumentDetail | null>("show_document", { workspaceId, id }),
  removeDocument: (workspaceId: string, id: string) =>
    invoke<boolean>("remove_document", { workspaceId, id }),
  ingestPath: (workspaceId: string, path: string) =>
    invoke<IngestSummary>("ingest_path", { workspaceId, path }),
  ingestPaths: (workspaceId: string, paths: string[]) =>
    invoke<IngestSummary>("ingest_paths", { workspaceId, paths }),
  ingestCancel: () => invoke<void>("ingest_cancel"),
  askDocuments: (req: {
    workspace_id: string;
    question: string;
    provider_id: string;
    model?: string;
    allow_general_knowledge?: boolean;
  }) => invoke<AskDocumentsResponse>("ask_documents", { request: req }),

  // Deident
  deidentText: (text: string) => invoke<DeidentResponse>("deident_text", { text }),

  // Providers
  listProviders: () => invoke<ProviderInfo[]>("list_providers"),
  setProviderKey: (id: string, apiKey: string) =>
    invoke<void>("set_provider_key", { id, apiKey }),
  testProvider: (id: string, prompt?: string) =>
    invoke<string>("test_provider", { id, prompt }),
  removeProviderKey: (id: string) =>
    invoke<void>("remove_provider_key", { id }),
  oauthAnthropicStart: () =>
    invoke<{ url: string; provider_id: string; instructions: string }>(
      "oauth_anthropic_start",
    ),
  oauthAnthropicComplete: (code: string) =>
    invoke<void>("oauth_anthropic_complete", { code }),
  // Opens the browser and spawns a background task that owns the
  // localhost:1455 listener. Returns the authorize URL so the UI can also
  // offer copy/open buttons — poll listProviders to detect completion,
  // call oauthOpenaiCancel to release the port.
  oauthOpenaiStart: () =>
    invoke<{ url: string; provider_id: string; instructions: string }>(
      "oauth_openai_start",
    ),
  oauthOpenaiCancel: () => invoke<void>("oauth_openai_cancel"),
  oauthLogout: (id: string) => invoke<void>("oauth_logout", { id }),

  // Verdict
  runCase: (req: {
    workspace_id: string;
    text: string;
    question: string;
    provider_id: string;
    model?: string;
  }) => invoke<CaseRunResponse>("run_case", { request: req }),
  listCases: (workspaceId: string, limit: number) =>
    invoke<CaseRecord[]>("list_cases", { workspaceId, limit }),
  showCase: (workspaceId: string, id: string) =>
    invoke<CaseDetail | null>("show_case", { workspaceId, id }),
  submitFeedback: (req: {
    workspace_id: string;
    case_id: string;
    kind: "accept" | "modify" | "reject";
    reason?: string;
  }) => invoke<void>("submit_feedback", { request: req }),
};

// ---------------------------------------------------------------------------
// Provider-slot helpers
//
// Conclave enforces a single active provider at the UI layer: at most one
// configured non-always-available provider may occupy the slot. Ollama is
// always available (local) and never counts.
// ---------------------------------------------------------------------------

export function activeProvider(list: ProviderInfo[]): ProviderInfo | null {
  return list.find((p) => p.configured && isOccupyingSlot(p.id)) ?? null;
}

export function connectedSlotProviders(list: ProviderInfo[]): ProviderInfo[] {
  return list.filter((p) => p.configured && isOccupyingSlot(p.id));
}

export function usableProviders(list: ProviderInfo[]): ProviderInfo[] {
  // A provider is usable if it's configured (API key set / OAuth completed)
  // OR if it's Ollama AND the local server is actually responding right now.
  // We rely on `available` here so Ollama only shows up when it's reachable
  // — we never advertise an AI the user can't actually call.
  return list.filter(
    (p) => (p.configured || p.id === "ollama") && p.available,
  );
}
