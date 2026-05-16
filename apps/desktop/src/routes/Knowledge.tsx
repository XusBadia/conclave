import { useEffect, useState } from "react";
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
      title: "Pick a file or directory to ingest",
    });
    if (!path) return;
    setIngesting(true);
    setIngestStatus("Embedding model warming up — first run downloads ~470 MB.");
    setError(null);
    try {
      const summary = await ipc.ingestPath(workspace.id, String(path));
      setIngestStatus(
        `Ingested ${summary.ingested} · skipped ${summary.skipped} · failed ${summary.failed}`,
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
      title: "Pick a folder to ingest (recursive)",
    });
    if (!path) return;
    setIngesting(true);
    setError(null);
    try {
      const summary = await ipc.ingestPath(workspace.id, String(path));
      setIngestStatus(
        `Ingested ${summary.ingested} · skipped ${summary.skipped} · failed ${summary.failed}`,
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
    if (!confirm("Remove this document and its chunks?")) return;
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
            title="Documents"
            subtitle={`${docs.length} documents in this workspace`}
            right={
              <div className="flex gap-2">
                <Button size="sm" onClick={ingest} loading={ingesting}>
                  Ingest file…
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={ingestFolder}
                  disabled={ingesting}
                >
                  Ingest folder…
                </Button>
                <Button size="sm" variant="ghost" onClick={refresh}>
                  Refresh
                </Button>
              </div>
            }
          />
          <CardBody className="p-0">
            {ingestStatus && (
              <div className="mx-5 mt-4 rounded-md border border-accent/40 bg-accent/5 px-3 py-2 text-[13px] text-accent">
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
                No documents yet. Ingest a PDF, DOCX, TXT, MD or HTML file to
                start.
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
                    Remove
                  </Button>
                </li>
              ))}
            </ul>
          </CardBody>
        </Card>
      </div>

      <Card>
        <CardHeader title="Vector search" subtitle="Top-K similarity over the workspace" />
        <CardBody className="space-y-4">
          <Field label="Query">
            <Input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && search()}
              placeholder="e.g. furosemida insuficiencia cardiaca"
            />
          </Field>
          <Button
            variant="primary"
            size="md"
            onClick={search}
            loading={searching}
            disabled={!query.trim()}
          >
            Search
          </Button>
          {hits && (
            <div className="space-y-3 pt-1">
              {hits.length === 0 && (
                <div className="text-[13px] text-ink-subtle">No hits.</div>
              )}
              {hits.map((h, i) => (
                <div
                  key={h.chunk_id}
                  className="rounded-md border border-border-subtle bg-bg p-3"
                >
                  <div className="mb-1 flex items-center justify-between text-[11px] text-ink-faint">
                    <span>#{i + 1} · distance {h.distance.toFixed(3)}</span>
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
