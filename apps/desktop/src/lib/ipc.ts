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
  // `subtask` flags providers that are restricted to non-clinical
  // utility flows (Apple Intelligence today). The picker filter in
  // Cases/Knowledge already hides them; we surface the kind so the
  // Settings card can render the right badge.
  kind: "standard" | "oauth" | "subtask";
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
  /** RFC3339. User-facing clinical date. Defaults to created_at on insert. */
  case_date: string;
  workspace_id: string;
  question: string;
  original_text: string;
  masked_text: string;
  deident_pipeline_id: string;
  status: "draft" | "completed" | "failed";
  /** Human-friendly identifier shown as the row title in the list — e.g.
   *  "Juan Pérez" or "CR-IA-011". Empty falls back to the question. */
  patient_label: string;
  /** When `status === "failed"`, the diagnostic message captured at run
   *  time. Surfaced in the detail view so the clinician sees *why* the
   *  committee aborted. Null otherwise. */
  latest_error: string | null;
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

export interface CaseAttachment {
  id: string;
  case_id: string;
  position: number;
  original_filename: string;
  stored_path: string;
  sha256: string;
  doc_type: "pdf" | "docx" | "txt" | "md" | "html" | "image";
  mime: string;
  extracted_text: string;
  needs_ocr: boolean;
  byte_size: number;
  created_at: string;
}

export interface CaseRunResponse {
  case: CaseRecord;
  verdict_record: VerdictRecord;
  verdict: Verdict;
  attachments: CaseAttachment[];
}

export interface CaseDetail {
  case: CaseRecord;
  verdict_record: VerdictRecord | null;
  verdict: Verdict | null;
}

export type DeliberationPhase =
  | "briefing"
  | "drafting"
  | "redteam"
  | "finalize";

export type DeliberationEvent =
  | {
      kind: "phase_started";
      phase: DeliberationPhase;
      case_id: string;
      batch_index: number | null;
    }
  | {
      kind: "phase_completed";
      phase: DeliberationPhase;
      output: string;
      elapsed_ms: number;
      case_id: string;
      batch_index: number | null;
    }
  | {
      kind: "phase_failed";
      phase: DeliberationPhase;
      error: string;
      elapsed_ms: number;
      case_id: string;
      batch_index: number | null;
    }
  | {
      kind: "done";
      verdict_json: string;
      case_id: string;
      batch_index: number | null;
    };

export interface DeliberationTrace {
  id: string;
  verdict_id: string;
  briefing_output: string | null;
  drafting_output: string | null;
  redteam_output: string | null;
  total_input_tokens: number;
  total_output_tokens: number;
  duration_ms: number;
  vision_used: boolean;
  created_at: string;
}

export interface BatchCaseInput {
  patient_label: string;
  text: string;
  question: string;
  attached_file_paths: string[];
}

export interface BatchRunSummary {
  completed: number;
  failed: number;
  cancelled: number;
}

export type BatchEvent =
  | { kind: "case_queued"; index: number; patient_label: string }
  | { kind: "case_started"; index: number; patient_label: string }
  | {
      kind: "case_completed";
      index: number;
      patient_label: string;
      case_id: string;
    }
  | {
      kind: "case_failed";
      index: number;
      patient_label: string;
      error: string;
    }
  | { kind: "case_cancelled"; index: number; patient_label: string }
  | {
      kind: "batch_done";
      completed: number;
      failed: number;
      cancelled: number;
    };

/**
 * Emitted by the backend the moment a case row is persisted as
 * `Draft` — before the LLM call begins. Drives the "case appears
 * immediately" UX in the cases list.
 */
export interface CaseDraftedEvent {
  case_id: string;
  workspace_id: string;
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
    attached_file_paths?: string[];
    patient_label?: string;
  }) => invoke<CaseRunResponse>("run_case", { request: req }),
  runCaseDeliberated: (req: {
    workspace_id: string;
    text: string;
    question: string;
    provider_id: string;
    model?: string;
    attached_file_paths?: string[];
    patient_label?: string;
  }) => invoke<CaseRunResponse>("run_case_deliberated", { request: req }),
  listCases: (workspaceId: string, limit: number) =>
    invoke<CaseRecord[]>("list_cases", { workspaceId, limit }),
  showCase: (workspaceId: string, id: string) =>
    invoke<CaseDetail | null>("show_case", { workspaceId, id }),
  listCaseAttachments: (workspaceId: string, caseId: string) =>
    invoke<CaseAttachment[]>("list_case_attachments", {
      workspaceId,
      caseId,
    }),
  getDeliberationTrace: (workspaceId: string, verdictId: string) =>
    invoke<DeliberationTrace | null>("get_deliberation_trace", {
      workspaceId,
      verdictId,
    }),
  submitFeedback: (req: {
    workspace_id: string;
    case_id: string;
    kind: "accept" | "modify" | "reject";
    reason?: string;
  }) => invoke<void>("submit_feedback", { request: req }),
  updateCaseDate: (req: {
    workspace_id: string;
    case_ids: string[];
    new_date: string;
  }) => invoke<void>("update_case_date", { request: req }),
  deleteCases: (req: { workspace_id: string; case_ids: string[] }) =>
    invoke<{ deleted: number }>("delete_cases", { request: req }),

  // Batch
  parseBatchFolder: (folderPath: string, defaultQuestion: string) =>
    invoke<BatchCaseInput[]>("parse_batch_folder", {
      folderPath,
      defaultQuestion,
    }),
  proposeCaseGrouping: (paths: string[], defaultQuestion: string) =>
    invoke<BatchCaseInput[]>("propose_case_grouping", {
      paths,
      defaultQuestion,
    }),
  runBatchCases: (req: {
    workspace_id: string;
    provider_id: string;
    model?: string;
    deliberative: boolean;
    cases: BatchCaseInput[];
  }) => invoke<BatchRunSummary>("run_batch_cases", { request: req }),
  batchCancel: () => invoke<void>("batch_cancel"),
  /** Re-run Apple Intelligence title generation for an existing case.
   *  Returns the new label on success or `null` when nothing changed
   *  (Apple Intelligence unavailable, timeout, empty response, …). */
  regenerateCaseLabel: (workspaceId: string, caseId: string) =>
    invoke<string | null>("regenerate_case_label", {
      workspaceId,
      caseId,
    }),

  // Drafts
  createDraftCases: (req: {
    workspace_id: string;
    cases: BatchCaseInput[];
  }) => invoke<CaseRecord[]>("create_draft_cases", { request: req }),
  runDraftCase: (req: {
    workspace_id: string;
    case_id: string;
    provider_id: string;
    model?: string;
    text?: string;
    question?: string;
  }) => invoke<CaseRunResponse>("run_draft_case", { request: req }),

  // Per-case cancel / retry
  /** Signal the backend to short-circuit an in-flight case run at the
   *  next phase boundary. Safe to call for unknown ids (no-op). */
  cancelCase: (caseId: string) =>
    invoke<void>("cancel_case", { caseId }),
  /** Reset a Failed case back to Draft so it can be retried without
   *  re-uploading its attachments. Returns the updated CaseRecord. */
  resetCaseToDraft: (workspaceId: string, caseId: string) =>
    invoke<CaseRecord>("reset_case_to_draft", { workspaceId, caseId }),
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
