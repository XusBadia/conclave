import { invoke } from "@tauri-apps/api/core";

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
  disclaimer: string;
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

export interface SearchHit {
  chunk_id: string;
  document_id: string;
  text: string;
  distance: number;
}

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
  searchWorkspace: (workspaceId: string, query: string, k: number) =>
    invoke<SearchHit[]>("search_workspace", { workspaceId, query, k }),

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
