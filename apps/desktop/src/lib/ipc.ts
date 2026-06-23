import { invoke } from "@tauri-apps/api/core";

import { isOccupyingSlot } from "./providers";
import { isConfigured, isReady } from "./providerStatus";

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

/**
 * Mirrors the Rust-side `ProviderStatus` enum in `commands.rs`. One of
 * six mutually exclusive states; the UI renders a single
 * `<ProviderStatusPill>` per provider rather than two independent flags
 * that could disagree (we used to ship "Connected" + "Reachable" green
 * badges next to an "authentication failed" banner — that contradiction
 * is exactly what this refactor kills).
 */
export type ProviderStatus =
  | "ready"
  | "expired"
  | "unreachable"
  | "not_configured"
  | "login_required"
  | "not_installed";

export interface ProviderInfo {
  id: string;
  status: ProviderStatus;
  default_model: string;
  requires_network: boolean;
  auth: "api-key" | "local" | "oauth" | "cli";
  // `subtask` flags providers that are restricted to non-clinical
  // utility flows (Apple Intelligence today). The picker filter in
  // Cases/Knowledge already hides them; we surface the kind so the
  // Settings card can render the right badge.
  kind: "standard" | "oauth" | "subtask";
  hint: string | null;
}

/**
 * Snapshot of the login-status probe (what command we ran, how it
 * exited, whether we trusted a fallback artifact). Surfaced under the
 * "Diagnóstico técnico" disclosure of the CLI setup panel so the next
 * "no detecta" report ships with enough detail to diagnose in a single
 * screenshot.
 */
export interface CliProbeDetails {
  logged_in: boolean;
  command: string | null;
  exit_code: number | null;
  stderr_excerpt: string;
  duration_ms: number | null;
  timed_out: boolean;
  /** When non-null, the artifact we trusted instead of the probe's exit
   * code (e.g. `~/.codex/auth.json`, `keychain`). */
  fallback_used: string | null;
  /** Names (NOT values) of environment variables that were set when we
   * spawned the probe. Helps debug launchd-vs-shell divergence. */
  env_keys_seen: string[];
  binary_mtime: number | null;
  binary_size: number | null;
}

/**
 * Diagnostic payload returned by the `cli_diagnostics` Tauri command.
 * Drives the in-Settings CLI setup panel. Includes the process `$PATH`
 * verbatim so the user can verify whether their unusual install dir is
 * visible to the bundled app — the most common failure mode on macOS is
 * a binary in `~/.local/bin` or `~/.nvm/.../bin` that launchd's bare
 * PATH doesn't include.
 */
export interface CliDiagnostics {
  binary_path: string | null;
  path_var: string;
  install_url: string;
  login_command: string;
  status: ProviderStatus;
  probe: CliProbeDetails;
  /** `true` when the user clicked "Marcar como conectado" for this
   * provider — Conclave then treats the binary as logged in regardless
   * of what the probe returned. */
  user_marked_ready: boolean;
}

export interface Verdict {
  case_summary: string;
  key_clinical_data: { label: string; value: string }[];
  applied_evidence: { ref: string; claim: string }[];
  primary_recommendation: { action: string; rationale: string };
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
  status:
    | "draft"
    | "review_ready"
    | "finalized"
    | "finalized_legacy"
    | "failed";
  /** Human-friendly identifier shown as the row title in the list — e.g.
   *  "Juan Pérez" or "CR-IA-011". Empty falls back to the question. */
  patient_label: string;
  /** When `status === "failed"`, the diagnostic message captured at run
   *  time. Surfaced in the detail view so the clinician sees *why* the
   *  committee aborted. Null otherwise. */
  latest_error: string | null;
  raw_text_sha256: string;
  raw_text_retention: RawTextRetention;
}

export type RawTextRetention =
  | "legacy_retained"
  | "temporary_draft"
  | "explicit_retained"
  | "discarded";

export type DataBoundaryMode = "local_only" | "deid_cloud" | "explicit_phi";

export type AuditPayloadMode = "none" | "fingerprint" | "preview" | "payload";

export interface PrivacySettings {
  default_data_boundary: DataBoundaryMode;
  purge_attachments_with_raw_text: boolean;
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
  data_boundary: DataBoundaryPreview;
}

export interface CaseDetail {
  case: CaseRecord;
  verdict_record: VerdictRecord | null;
  verdict: Verdict | null;
  attachments: CaseAttachment[];
  audit: AuditRunRecord | null;
  review: ReviewMetadataRecord | null;
}

export interface AuditRunRecord {
  id: string;
  case_id: string;
  verdict_id: string | null;
  provider_id: string;
  model: string;
  data_boundary_mode: DataBoundaryMode;
  payload_mode: AuditPayloadMode;
  active_skill_id: string | null;
  started_at: string;
  completed_at: string | null;
  latency_ms: number;
  input_tokens: number;
  output_tokens: number;
  prompt_sha256: string;
  output_sha256: string;
  evidence_refs: string[];
  past_cases_refs: string[];
  online_evidence_refs: string[];
  attachment_refs: string[];
  raw_text_retention: RawTextRetention;
  status: string;
  error: string | null;
}

export interface AuditStatus {
  run_count: number;
  payload_mode: AuditPayloadMode;
  retained_raw_cases: number;
  legacy_retained_cases: number;
}

export interface ReviewMetadataRecord {
  case_id: string;
  verdict_id: string;
  decision: "accept" | "modify" | "reject";
  reviewer_name: string | null;
  reviewer_role: string | null;
  note: string | null;
  final_verdict_json: string | null;
  diff_summary: string | null;
  reviewed_at: string;
}

export interface DataBoundaryPreview {
  mode: DataBoundaryMode;
  provider_id: string;
  provider_requires_network: boolean;
  sends_masked_text: boolean;
  sends_raw_text: boolean;
  sends_images: boolean;
  stores_raw_text: boolean;
  retains_attachment_files: boolean;
  uses_online_evidence: boolean;
  blocked_reason: string | null;
}

export interface Skill {
  id: string;
  title: string;
  description: string;
  recommended_workflow: string;
  allowed_modes: string[];
  body: string;
  source: "built_in" | "user" | "workspace";
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
      kind: "phase_retrying";
      phase: DeliberationPhase;
      attempt: number;
      reason: string;
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

  // Providers
  listProviders: (opts?: { forceRefresh?: boolean }) =>
    invoke<ProviderInfo[]>("list_providers", {
      forceRefresh: opts?.forceRefresh ?? false,
    }),
  setProviderKey: (id: string, apiKey: string) =>
    invoke<void>("set_provider_key", { id, apiKey }),
  testProvider: (id: string, prompt?: string) =>
    invoke<string>("test_provider", { id, prompt }),
  removeProviderKey: (id: string) =>
    invoke<void>("remove_provider_key", { id }),
  cliDiagnostics: (id: "claude-cli" | "codex-cli") =>
    invoke<CliDiagnostics>("cli_diagnostics", { id }),
  /** Invalidate the process-wide which() cache for both CLI providers
   *  so the next listProviders/cliDiagnostics call re-walks $PATH. Use
   *  after the user installs the CLI in a terminal while Conclave is
   *  open — they shouldn't have to restart the app to be detected. */
  redetectCliBinaries: () => invoke<void>("redetect_cli_binaries"),
  /** Toggle the manual "I'm logged in, trust me" override for one CLI
   *  provider. Use when auto-detection fails despite the user being
   *  logged in (Keychain ACL quirks, launchd env divergence). The flag
   *  persists in `conclave.toml` so the user only needs to declare it
   *  once; passing `value: false` removes the override. */
  setCliLoginOverride: (
    id: "claude-cli" | "codex-cli",
    value: boolean,
  ) => invoke<void>("set_cli_login_override", { id, value }),
  privacySettings: () => invoke<PrivacySettings>("privacy_settings"),
  setPrivacySettings: (settings: PrivacySettings) =>
    invoke<PrivacySettings>("set_privacy_settings", { settings }),
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
    data_boundary_mode?: DataBoundaryMode;
    allow_phi_payload?: boolean;
    retain_raw_text?: boolean;
    active_skill_id?: string;
    use_online_evidence?: boolean;
  }) => invoke<CaseRunResponse>("run_case", { request: req }),
  runCaseDeliberated: (req: {
    workspace_id: string;
    text: string;
    question: string;
    provider_id: string;
    model?: string;
    attached_file_paths?: string[];
    patient_label?: string;
    data_boundary_mode?: DataBoundaryMode;
    allow_phi_payload?: boolean;
    retain_raw_text?: boolean;
    active_skill_id?: string;
    use_online_evidence?: boolean;
  }) => invoke<CaseRunResponse>("run_case_deliberated", { request: req }),
  previewDataBoundary: (req: {
    workspace_id: string;
    text: string;
    question: string;
    provider_id: string;
    model?: string;
    attached_file_paths?: string[];
    patient_label?: string;
    data_boundary_mode?: DataBoundaryMode;
    allow_phi_payload?: boolean;
    retain_raw_text?: boolean;
    active_skill_id?: string;
    use_online_evidence?: boolean;
  }) => invoke<DataBoundaryPreview>("preview_data_boundary", { request: req }),
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
    reviewer_name?: string;
    reviewer_role?: string;
    final_verdict_json?: string;
  }) => invoke<void>("submit_feedback", { request: req }),
  purgeCasePhi: (workspaceId: string, caseId: string) =>
    invoke<CaseRecord>("purge_case_phi", { workspaceId, caseId }),
  purgeCaseAttachments: (workspaceId: string, caseId: string) =>
    invoke<number>("purge_case_attachments", { workspaceId, caseId }),
  auditStatus: (workspaceId: string) =>
    invoke<AuditStatus>("audit_status", { workspaceId }),
  listAuditRuns: (workspaceId: string, limit: number) =>
    invoke<AuditRunRecord[]>("list_audit_runs", { workspaceId, limit }),
  exportAuditRuns: (workspaceId: string) =>
    invoke<AuditRunRecord[]>("export_audit_runs", { workspaceId }),
  listSkills: (workspaceId: string) =>
    invoke<Skill[]>("list_skills", { workspaceId }),
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
    data_boundary_mode?: DataBoundaryMode;
    allow_phi_payload?: boolean;
    retain_raw_text?: boolean;
    active_skill_id?: string;
    use_online_evidence?: boolean;
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
  // "Active" = the user has a credential for a slot-occupying provider.
  // Whether it's actually reachable right now is a *separate* question
  // handled by the status pill; for the empty-state vs active-card
  // decision in Settings we only care that something is connected.
  return list.find((p) => isConfigured(p) && isOccupyingSlot(p.id)) ?? null;
}

export function connectedSlotProviders(list: ProviderInfo[]): ProviderInfo[] {
  return list.filter((p) => isConfigured(p) && isOccupyingSlot(p.id));
}

export function usableProviders(list: ProviderInfo[]): ProviderInfo[] {
  // "Usable right now" means `status === "ready"`: the credential is
  // present AND (for OAuth / Ollama) the upstream probe just succeeded.
  // We never advertise an AI the user can't actually call.
  return list.filter(isReady);
}
