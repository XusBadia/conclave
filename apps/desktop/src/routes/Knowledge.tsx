import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

import { Button } from "../components/Button";
import { Card, CardBody, CardHeader } from "../components/Card";
import { Field, Textarea } from "../components/Field";
import { cn } from "../lib/cn";
import {
  ipc,
  usableProviders,
  type AskDocumentsResponse,
  type DocumentRecord,
  type IngestProgressEvent,
  type IngestStage,
  type ProviderInfo,
  type Workspace,
} from "../lib/ipc";

/**
 * Pull the optional `⚠️ …` opening paragraph out of an LLM answer so the
 * UI can render it as a separate visual badge instead of as the first
 * paragraph of the answer body. We split on the first blank line because
 * the system prompt requires the warning to be its own paragraph.
 */
function splitAnswerDisclaimer(text: string): {
  disclaimer: string | null;
  body: string;
} {
  const trimmed = text.trimStart();
  if (!trimmed.startsWith("⚠️")) return { disclaimer: null, body: text };
  const blank = trimmed.indexOf("\n\n");
  if (blank === -1) {
    return { disclaimer: trimmed.replace(/^⚠️\s*/, "").trim(), body: "" };
  }
  return {
    disclaimer: trimmed.slice(0, blank).replace(/^⚠️\s*/, "").trim(),
    body: trimmed.slice(blank + 2).trimStart(),
  };
}

type FileState =
  | { phase: "queued" }
  | { phase: "active"; stage: IngestStage; percent: number }
  | { phase: "done" }
  | { phase: "failed"; error: string }
  | { phase: "skipped"; reason: string };

export function KnowledgePage({ workspace }: { workspace: Workspace }) {
  const { t } = useTranslation();
  const [docs, setDocs] = useState<DocumentRecord[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [ingestStatus, setIngestStatus] = useState<string | null>(null);
  const [ingesting, setIngesting] = useState(false);
  const [dropActive, setDropActive] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const [fileStates, setFileStates] = useState<Map<string, FileState>>(
    () => new Map(),
  );
  const menuRef = useRef<HTMLDivElement>(null);
  const clearTimer = useRef<number | undefined>(undefined);

  // --- Q&A panel state ----------------------------------------------------
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [providerId, setProviderId] = useState<string>("");
  const [question, setQuestion] = useState("");
  const [askResp, setAskResp] = useState<AskDocumentsResponse | null>(null);
  const [asking, setAsking] = useState(false);
  const [askError, setAskError] = useState<string | null>(null);
  // When true, the model may use its general training knowledge for parts
  // the documents don't cover — provided it explicitly flags those parts.
  // No live web access is involved.
  const [allowGeneralKnowledge, setAllowGeneralKnowledge] = useState(false);

  const usable = useMemo(() => usableProviders(providers), [providers]);
  const askParts = useMemo(
    () =>
      askResp
        ? splitAnswerDisclaimer(askResp.answer)
        : { disclaimer: null, body: "" },
    [askResp],
  );

  const refresh = async () => {
    setLoading(true);
    setError(null);
    try {
      setDocs(await ipc.listDocuments(workspace.id));
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    refresh();
  }, [workspace.id]);

  // Load providers once — used by Q&A panel.
  useEffect(() => {
    (async () => {
      try {
        const list = await ipc.listProviders();
        setProviders(list);
      } catch {
        /* swallowed; UI shows no-provider state */
      }
    })();
  }, []);

  // Auto-select a provider when the usable list is available.
  useEffect(() => {
    if (providerId) return;
    if (usable.length === 0) return;
    const ollama = usable.find((p) => p.id === "ollama" && p.available);
    setProviderId(ollama?.id ?? usable[0].id);
  }, [usable, providerId]);

  const ingestPaths = async (paths: string[]) => {
    if (clearTimer.current) {
      window.clearTimeout(clearTimer.current);
      clearTimer.current = undefined;
    }
    const initial = new Map<string, FileState>();
    for (const p of paths) initial.set(p, { phase: "queued" });
    setFileStates(initial);
    setIngesting(true);
    setIngestStatus(t("knowledge.ingest_warming"));
    setError(null);
    try {
      const summary = await ipc.ingestPaths(workspace.id, paths);
      setIngestStatus(
        t("knowledge.ingest_summary", {
          ingested: summary.ingested,
          skipped: summary.skipped,
          failed: summary.failed,
        }),
      );
      await refresh();
    } catch (e) {
      setError(String(e));
      setIngestStatus(null);
    } finally {
      setIngesting(false);
      clearTimer.current = window.setTimeout(
        () => setFileStates(new Map()),
        2500,
      );
    }
  };

  const pickFile = async () => {
    const path = await openDialog({
      multiple: false,
      directory: false,
      title: t("knowledge.pick_file_title"),
    });
    if (!path) return;
    await ingestPaths([String(path)]);
  };

  const pickFolder = async () => {
    const path = await openDialog({
      multiple: false,
      directory: true,
      title: t("knowledge.pick_folder_title"),
    });
    if (!path) return;
    await ingestPaths([String(path)]);
  };

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    (async () => {
      const webview = getCurrentWebviewWindow();
      const fn = await webview.onDragDropEvent((event) => {
        if (event.payload.type === "enter" || event.payload.type === "over") {
          setDropActive(true);
        } else if (event.payload.type === "leave") {
          setDropActive(false);
        } else if (event.payload.type === "drop") {
          setDropActive(false);
          const paths = event.payload.paths;
          if (paths.length > 0) void ingestPaths(paths);
        }
      });
      if (cancelled) fn();
      else unlisten = fn;
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [workspace.id]);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    (async () => {
      unlisten = await listen<IngestProgressEvent>(
        "ingest:progress",
        (msg) => {
          const ev = msg.payload;
          setFileStates((prev) => {
            const next = new Map(prev);
            if (ev.kind === "progress") {
              next.set(ev.path, {
                phase: "active",
                stage: ev.stage,
                percent: ev.percent,
              });
            } else if (ev.kind === "ingested") {
              next.set(ev.path, { phase: "done" });
            } else if (ev.kind === "failed") {
              next.set(ev.path, { phase: "failed", error: ev.error });
            } else if (ev.kind === "skipped") {
              next.set(ev.path, { phase: "skipped", reason: ev.reason });
            } else if (ev.kind === "starting") {
              if (!next.has(ev.path)) next.set(ev.path, { phase: "queued" });
            }
            return next;
          });
        },
      );
    })();
    return () => unlisten?.();
  }, []);

  useEffect(() => {
    if (!menuOpen) return;
    const handler = (e: MouseEvent) => {
      if (!menuRef.current?.contains(e.target as Node)) setMenuOpen(false);
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [menuOpen]);

  const remove = async (id: string) => {
    if (!confirm(t("knowledge.confirm_remove"))) return;
    try {
      await ipc.removeDocument(workspace.id, id);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const cancelIngest = () => {
    void ipc.ingestCancel();
  };

  const ask = async () => {
    if (!question.trim() || !providerId) return;
    setAsking(true);
    setAskError(null);
    setAskResp(null);
    try {
      const resp = await ipc.askDocuments({
        workspace_id: workspace.id,
        question: question.trim(),
        provider_id: providerId,
        allow_general_knowledge: allowGeneralKnowledge,
      });
      setAskResp(resp);
    } catch (e) {
      setAskError(String(e));
    } finally {
      setAsking(false);
    }
  };

  const renderStageLabel = (st: FileState): string => {
    if (st.phase === "active") return t(`knowledge.stage.${st.stage}`);
    if (st.phase === "done") return t("knowledge.stage.done");
    if (st.phase === "failed") return t("knowledge.stage.failed");
    if (st.phase === "skipped") return t("knowledge.stage.skipped");
    return t("knowledge.stage.queued");
  };

  const progressWidth = (st: FileState): string => {
    if (st.phase === "active") return `${st.percent}%`;
    if (st.phase === "done") return "100%";
    if (st.phase === "failed" || st.phase === "skipped") return "100%";
    return "0%";
  };

  return (
    <div className="mx-auto grid w-full max-w-6xl grid-cols-1 gap-5 p-6 lg:grid-cols-[minmax(0,1fr)_420px]">
      {dropActive && (
        <div className="pointer-events-none fixed inset-0 z-50 flex items-center justify-center bg-bg/80 backdrop-blur-sm">
          <div className="rounded-xl border-2 border-dashed border-accent px-10 py-8 text-center text-[15px] font-medium text-ink">
            {t("knowledge.dropzone_overlay")}
          </div>
        </div>
      )}

      <div className="min-w-0 space-y-5">
        <Card>
          <CardHeader
            title={t("knowledge.documents")}
            subtitle={t("knowledge.documents_count", { count: docs.length })}
            right={
              <div className="flex items-center gap-2">
                <div className="relative" ref={menuRef}>
                  <Button
                    size="sm"
                    onClick={() => setMenuOpen((o) => !o)}
                    loading={ingesting}
                    aria-haspopup="menu"
                    aria-expanded={menuOpen}
                  >
                    {t("knowledge.add_button")}
                  </Button>
                  {menuOpen && (
                    <div
                      role="menu"
                      className="absolute right-0 top-full z-20 mt-1 min-w-[160px] overflow-hidden rounded-md border border-border bg-bg shadow-lg"
                    >
                      <button
                        type="button"
                        role="menuitem"
                        className="block w-full px-3 py-2 text-left text-[13px] text-ink hover:bg-surface"
                        onClick={() => {
                          setMenuOpen(false);
                          void pickFile();
                        }}
                      >
                        {t("knowledge.add_file")}
                      </button>
                      <button
                        type="button"
                        role="menuitem"
                        className="block w-full px-3 py-2 text-left text-[13px] text-ink hover:bg-surface"
                        onClick={() => {
                          setMenuOpen(false);
                          void pickFolder();
                        }}
                      >
                        {t("knowledge.add_folder")}
                      </button>
                    </div>
                  )}
                </div>
                <Button size="sm" variant="ghost" onClick={refresh}>
                  {t("common.refresh")}
                </Button>
              </div>
            }
          />
          <CardBody className="p-0">
            {ingestStatus && (
              <div className="mx-5 mt-4 border border-border bg-surface px-3 py-2 text-[13px] text-ink-dim">
                {ingestStatus}
              </div>
            )}
            {error && (
              <div className="mx-5 mt-4 rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-[13px] text-danger">
                {error}
              </div>
            )}
            {fileStates.size > 0 && (
              <div className="mx-5 mt-3 space-y-2">
                {Array.from(fileStates.entries()).map(([path, st]) => {
                  const name = path.split("/").pop() ?? path;
                  const failed =
                    st.phase === "failed" || st.phase === "skipped";
                  return (
                    <div
                      key={path}
                      className="rounded-md border border-border-subtle bg-bg p-2"
                    >
                      <div className="flex items-center justify-between gap-2">
                        <span
                          className="min-w-0 flex-1 truncate text-[12px] text-ink"
                          title={path}
                        >
                          {name}
                        </span>
                        <span className="shrink-0 text-[11px] text-ink-faint">
                          {renderStageLabel(st)}
                        </span>
                      </div>
                      <div className="mt-1.5 h-1 w-full overflow-hidden rounded bg-surface">
                        <div
                          className={cn(
                            "h-full transition-all duration-200",
                            failed ? "bg-danger" : "bg-accent",
                          )}
                          style={{ width: progressWidth(st) }}
                        />
                      </div>
                    </div>
                  );
                })}
                {ingesting && (
                  <button
                    type="button"
                    className="text-[11px] text-ink-faint underline-offset-2 hover:text-ink hover:underline"
                    onClick={cancelIngest}
                  >
                    {t("knowledge.cancel_ingest")}
                  </button>
                )}
              </div>
            )}
            {docs.length === 0 && !loading && fileStates.size === 0 && (
              <div className="px-6 py-10 text-center text-[13px] text-ink-subtle">
                {t("knowledge.empty_docs")}
              </div>
            )}
            {docs.length > 0 && fileStates.size === 0 && (
              <div className="px-5 pt-3 text-[12px] text-ink-faint">
                {t("knowledge.dropzone_hint")}
              </div>
            )}
            <ul className="divide-y divide-border-subtle">
              {docs.map((d) => (
                <li
                  key={d.id}
                  className="flex min-w-0 items-start justify-between gap-4 px-5 py-3.5"
                >
                  <div className="min-w-0 flex-1">
                    <div
                      className="line-clamp-2 break-words text-[14px] font-medium leading-snug text-ink"
                      title={d.title || d.source_path}
                    >
                      {d.title || d.source_path.split("/").pop()}
                    </div>
                    <div className="mt-1 flex items-center gap-2 text-[11px] text-ink-faint">
                      <span className="rounded bg-surface px-1.5 py-0.5 uppercase tracking-wider">
                        {d.doc_type}
                      </span>
                      <span className="rounded bg-surface px-1.5 py-0.5">
                        {d.status}
                      </span>
                    </div>
                  </div>
                  <Button
                    size="sm"
                    variant="danger"
                    onClick={() => remove(d.id)}
                  >
                    {t("common.remove")}
                  </Button>
                </li>
              ))}
            </ul>
          </CardBody>
        </Card>
      </div>

      <Card>
        <CardHeader
          title={t("knowledge.qa.title")}
          subtitle={t("knowledge.qa.subtitle")}
        />
        <CardBody className="space-y-4">
          {usable.length === 0 ? (
            <div className="rounded-md border border-border-subtle bg-bg p-3 text-[13px] text-ink-subtle">
              {t("knowledge.qa.no_provider")}
            </div>
          ) : (
            <>
              {usable.length > 1 ? (
                <Field label={t("knowledge.qa.provider")}>
                  <select
                    className="block w-full rounded-lg border border-border bg-bg px-3 py-2 text-sm text-ink shadow-soft transition focus:border-accent focus:outline-none focus:ring-conclave"
                    value={providerId}
                    onChange={(e) => setProviderId(e.target.value)}
                  >
                    {usable.map((p) => (
                      <option key={p.id} value={p.id}>
                        {p.id}
                        {p.default_model ? ` · ${p.default_model}` : ""}
                      </option>
                    ))}
                  </select>
                </Field>
              ) : (
                <div className="text-[11px] uppercase tracking-wider text-ink-faint">
                  {t("knowledge.qa.using_provider", {
                    id: usable[0]?.id ?? "",
                  })}
                </div>
              )}
              <Field label={t("knowledge.qa.field_question")}>
                <Textarea
                  rows={3}
                  className="resize-none font-sans leading-snug"
                  value={question}
                  onChange={(e) => setQuestion(e.target.value)}
                  placeholder={t("knowledge.qa.field_question_placeholder")}
                />
              </Field>
              <label className="flex cursor-pointer items-start gap-2 text-[12px] leading-snug text-ink-dim">
                <input
                  type="checkbox"
                  className="mt-0.5"
                  checked={allowGeneralKnowledge}
                  onChange={(e) =>
                    setAllowGeneralKnowledge(e.target.checked)
                  }
                />
                <span>
                  {t("knowledge.qa.general_knowledge_label")}
                  <span className="mt-0.5 block text-[11px] text-ink-faint">
                    {t("knowledge.qa.general_knowledge_hint")}
                  </span>
                </span>
              </label>
              <Button
                variant="primary"
                size="md"
                onClick={ask}
                loading={asking}
                disabled={!question.trim() || !providerId || docs.length === 0}
              >
                {t("knowledge.qa.ask_button")}
              </Button>
              {docs.length === 0 && (
                <div className="text-[12px] text-ink-faint">
                  {t("knowledge.qa.empty_workspace")}
                </div>
              )}
            </>
          )}
          {askError && (
            <div className="rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-[13px] text-danger">
              {askError}
            </div>
          )}
          {askResp && (
            <div className="space-y-3">
              {askParts.disclaimer && (
                <div className="flex items-start gap-2 rounded-md border border-warn/40 bg-warn/10 px-2.5 py-1.5 text-[11px] leading-snug text-warn">
                  <span aria-hidden>⚠️</span>
                  <span>{askParts.disclaimer}</span>
                </div>
              )}
              <div className="rounded-md border border-border-subtle bg-bg p-3">
                <div className="text-[13px] leading-relaxed text-ink">
                  <ReactMarkdown
                    remarkPlugins={[remarkGfm]}
                        components={{
                          p: ({ children }) => (
                            <p className="my-2 first:mt-0 last:mb-0">
                              {children}
                            </p>
                          ),
                          strong: ({ children }) => (
                            <strong className="font-semibold text-ink">
                              {children}
                            </strong>
                          ),
                          em: ({ children }) => (
                            <em className="italic">{children}</em>
                          ),
                          ul: ({ children }) => (
                            <ul className="my-2 list-disc space-y-1 pl-5">
                              {children}
                            </ul>
                          ),
                          ol: ({ children }) => (
                            <ol className="my-2 list-decimal space-y-1 pl-5">
                              {children}
                            </ol>
                          ),
                          li: ({ children }) => (
                            <li className="leading-snug">{children}</li>
                          ),
                          h1: ({ children }) => (
                            <h3 className="mb-1 mt-3 text-[14px] font-semibold first:mt-0">
                              {children}
                            </h3>
                          ),
                          h2: ({ children }) => (
                            <h3 className="mb-1 mt-3 text-[14px] font-semibold first:mt-0">
                              {children}
                            </h3>
                          ),
                          h3: ({ children }) => (
                            <h3 className="mb-1 mt-3 text-[14px] font-semibold first:mt-0">
                              {children}
                            </h3>
                          ),
                          code: ({ children }) => (
                            <code className="rounded bg-surface px-1 py-0.5 font-mono text-[12px]">
                              {children}
                            </code>
                          ),
                          a: ({ children, href }) => (
                            <a
                              href={href}
                              target="_blank"
                              rel="noreferrer"
                              className="underline underline-offset-2 hover:text-ink"
                            >
                              {children}
                            </a>
                          ),
                          table: ({ children }) => (
                            <div className="my-2 overflow-x-auto">
                              <table className="w-full border-collapse text-[12px]">
                                {children}
                              </table>
                            </div>
                          ),
                          thead: ({ children }) => (
                            <thead className="bg-surface">{children}</thead>
                          ),
                          th: ({ children }) => (
                            <th className="border border-border-subtle px-2 py-1 text-left font-semibold">
                              {children}
                            </th>
                          ),
                          td: ({ children }) => (
                            <td className="border border-border-subtle px-2 py-1 align-top">
                              {children}
                            </td>
                          ),
                          blockquote: ({ children }) => (
                            <blockquote className="my-2 border-l-2 border-border-subtle pl-3 italic text-ink-dim">
                              {children}
                            </blockquote>
                          ),
                        }}
                  >
                    {askParts.body}
                  </ReactMarkdown>
                </div>
                <div className="mt-2 text-[10px] uppercase tracking-wider text-ink-faint">
                  {t("knowledge.qa.answered_by", { model: askResp.model })}
                </div>
              </div>
              {askResp.sources.length > 0 && (
                <div>
                  <div className="mb-1.5 text-[11px] uppercase tracking-wider text-ink-faint">
                    {t("knowledge.qa.sources_label")}
                  </div>
                  <div className="space-y-2">
                    {askResp.sources.map((s) => (
                      <div
                        key={s.chunk_id}
                        className="rounded-md border border-border-subtle bg-bg p-2.5"
                      >
                        <div
                          className="mb-1 truncate text-[11px] text-ink-faint"
                          title={s.document_title}
                        >
                          [{s.index}] {s.document_title}
                        </div>
                        <p className="line-clamp-3 text-[12px] leading-snug text-ink-dim">
                          {s.snippet}
                        </p>
                      </div>
                    ))}
                  </div>
                </div>
              )}
              {askResp.web_sources.length > 0 && (
                <div>
                  <div className="mb-1.5 text-[11px] uppercase tracking-wider text-ink-faint">
                    {t("knowledge.qa.web_sources_label")}
                  </div>
                  <div className="space-y-2">
                    {askResp.web_sources.map((w, i) => (
                      <a
                        key={`${w.url}-${i}`}
                        href={w.url}
                        target="_blank"
                        rel="noreferrer"
                        className="block rounded-md border border-border-subtle bg-bg p-2.5 hover:bg-surface"
                      >
                        <div
                          className="mb-0.5 truncate text-[12px] font-medium text-ink underline underline-offset-2"
                          title={w.title || w.url}
                        >
                          {w.title || w.url}
                        </div>
                        <div
                          className="truncate text-[11px] text-ink-faint"
                          title={w.url}
                        >
                          {w.url}
                        </div>
                      </a>
                    ))}
                  </div>
                </div>
              )}
            </div>
          )}
        </CardBody>
      </Card>
    </div>
  );
}
