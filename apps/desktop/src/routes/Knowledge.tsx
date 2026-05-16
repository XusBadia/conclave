import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open as openDialog } from "@tauri-apps/plugin-dialog";

import { Button } from "../components/Button";
import { Card, CardBody, CardHeader } from "../components/Card";
import { Field, Input } from "../components/Field";
import {
  ipc,
  type DocumentRecord,
  type SearchHit,
  type Workspace,
} from "../lib/ipc";

export function KnowledgePage({ workspace }: { workspace: Workspace }) {
  const { t } = useTranslation();
  const [docs, setDocs] = useState<DocumentRecord[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [ingestStatus, setIngestStatus] = useState<string | null>(null);
  const [ingesting, setIngesting] = useState(false);
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[] | null>(null);
  const [searching, setSearching] = useState(false);

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

  const ingest = async () => {
    const path = await openDialog({
      multiple: false,
      directory: false,
      title: t("knowledge.pick_file_title"),
    });
    if (!path) return;
    setIngesting(true);
    setIngestStatus(t("knowledge.ingest_warming"));
    setError(null);
    try {
      const summary = await ipc.ingestPath(workspace.id, String(path));
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
    }
  };

  const ingestFolder = async () => {
    const path = await openDialog({
      multiple: false,
      directory: true,
      title: t("knowledge.pick_folder_title"),
    });
    if (!path) return;
    setIngesting(true);
    setError(null);
    try {
      const summary = await ipc.ingestPath(workspace.id, String(path));
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
    } finally {
      setIngesting(false);
    }
  };

  const search = async () => {
    if (!query.trim()) return;
    setSearching(true);
    setError(null);
    try {
      setHits(await ipc.searchWorkspace(workspace.id, query.trim(), 8));
    } catch (e) {
      setError(String(e));
    } finally {
      setSearching(false);
    }
  };

  const remove = async (id: string) => {
    if (!confirm(t("knowledge.confirm_remove"))) return;
    try {
      await ipc.removeDocument(workspace.id, id);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="mx-auto grid w-full max-w-6xl grid-cols-1 gap-5 p-6 lg:grid-cols-[1fr,420px]">
      <div className="space-y-5">
        <Card>
          <CardHeader
            title={t("knowledge.documents")}
            subtitle={t("knowledge.documents_count", { count: docs.length })}
            right={
              <div className="flex gap-2">
                <Button size="sm" onClick={ingest} loading={ingesting}>
                  {t("knowledge.ingest_file")}
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={ingestFolder}
                  disabled={ingesting}
                >
                  {t("knowledge.ingest_folder")}
                </Button>
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
            {docs.length === 0 && !loading && (
              <div className="px-6 py-10 text-center text-[13px] text-ink-subtle">
                {t("knowledge.empty_docs")}
              </div>
            )}
            <ul className="divide-y divide-border-subtle">
              {docs.map((d) => (
                <li
                  key={d.id}
                  className="flex items-start justify-between gap-4 px-5 py-3.5"
                >
                  <div className="min-w-0">
                    <div className="truncate text-[14px] font-medium text-ink">
                      {d.title || d.source_path.split("/").pop()}
                    </div>
                    <div className="mt-0.5 truncate text-[12px] text-ink-faint">
                      <span className="font-mono">{d.id}</span>
                      <span className="ml-2 rounded bg-surface px-1.5 py-0.5 text-ink-subtle">
                        {d.doc_type}
                      </span>
                      <span className="ml-1 rounded bg-surface px-1.5 py-0.5 text-ink-subtle">
                        {d.status}
                      </span>
                    </div>
                  </div>
                  <Button size="sm" variant="danger" onClick={() => remove(d.id)}>
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
          title={t("knowledge.search_title")}
          subtitle={t("knowledge.search_subtitle")}
        />
        <CardBody className="space-y-4">
          <Field label={t("knowledge.field_query")}>
            <Input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && search()}
              placeholder={t("knowledge.field_query_placeholder")}
            />
          </Field>
          <Button
            variant="primary"
            size="md"
            onClick={search}
            loading={searching}
            disabled={!query.trim()}
          >
            {t("knowledge.search_button")}
          </Button>
          {hits && (
            <div className="space-y-3 pt-1">
              {hits.length === 0 && (
                <div className="text-[13px] text-ink-subtle">
                  {t("knowledge.no_hits")}
                </div>
              )}
              {hits.map((h, i) => (
                <div
                  key={h.chunk_id}
                  className="rounded-md border border-border-subtle bg-bg p-3"
                >
                  <div className="mb-1 flex items-center justify-between text-[11px] text-ink-faint">
                    <span>
                      {t("knowledge.hit_meta", {
                        index: i + 1,
                        distance: h.distance.toFixed(3),
                      })}
                    </span>
                    <span className="font-mono">{h.chunk_id}</span>
                  </div>
                  <p className="line-clamp-4 text-[13px] leading-snug text-ink-dim">
                    {h.text}
                  </p>
                </div>
              ))}
            </div>
          )}
        </CardBody>
      </Card>
    </div>
  );
}
