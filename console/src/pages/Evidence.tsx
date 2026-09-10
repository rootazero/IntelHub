// Evidence browser: document list (embedding status, dedupe) → detail.

import { useCallback, useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { api } from "../api";
import { useEnum, useT } from "../i18n";
import { Empty, ErrorBox, Loading, Panel, TimeAgo } from "../ui";

interface Doc {
  document_id: string;
  url: string;
  title?: string | null;
  content_hash: string;
  embedding_status: string;
  retrieved_at: string;
}

const EMB_TONE: Record<string, string> = {
  DONE: "text-ok",
  PENDING: "text-warn",
  RUNNING: "text-warn",
  SKIPPED: "text-dim",
  FAILED: "text-crit",
};

export default function Evidence() {
  const { t } = useT();
  const en = useEnum();
  const [items, setItems] = useState<Doc[]>([]);
  const [total, setTotal] = useState(0);
  const [error, setError] = useState<Error | null>(null);
  const [loading, setLoading] = useState(true);
  const [q, setQ] = useState("");
  const [status, setStatus] = useState("");
  const [offset, setOffset] = useState(0);
  const limit = 25;

  const load = useCallback(() => {
    const params = new URLSearchParams({ limit: String(limit), offset: String(offset) });
    if (q) params.set("q", q);
    if (status) params.set("status", status);
    api<{ total: number; items: Doc[] }>(`/api/v1/documents?${params}`)
      .then((r) => { setItems(r.items ?? []); setTotal(r.total); })
      .catch(setError)
      .finally(() => setLoading(false));
  }, [q, status, offset]);
  useEffect(load, [load]);

  return (
    <div className="p-4">
      <Panel
        title={t("evidence.title", { total })}
        right={
          <div className="flex gap-1.5 text-[11px]">
            <input value={q} onChange={(e) => { setOffset(0); setQ(e.target.value); }} placeholder={t("evidence.filterPh")}
              className="w-48 rounded border border-edge bg-base px-1.5 py-0.5" />
            <select value={status} onChange={(e) => { setOffset(0); setStatus(e.target.value); }}
              className="rounded border border-edge bg-base px-1.5 py-0.5">
              <option value="">{t("evidence.anyEmbedding")}</option>
              {["DONE", "PENDING", "SKIPPED", "FAILED"].map((s) => <option key={s} value={s}>{en("emb", s)}</option>)}
            </select>
          </div>
        }
      >
        {error && <ErrorBox error={error} />}
        {loading ? <Loading /> : items.length === 0 ? <Empty label={t("evidence.noDocs")} /> : (
          <>
            <table className="w-full text-xs">
              <thead><tr className="text-left text-[10px] uppercase text-dim">
                <th className="pb-1">{t("evidence.thDocument")}</th><th className="pb-1">{t("evidence.thHash")}</th><th className="pb-1">{t("evidence.thEmbedding")}</th><th className="pb-1">{t("evidence.thRetrieved")}</th>
              </tr></thead>
              <tbody>
                {items.map((d) => (
                  <tr key={d.document_id} className="border-t border-edge/50">
                    <td className="max-w-xl truncate py-1 pr-2">
                      <Link className="text-accent hover:underline" to={`/evidence/${d.document_id}`}>{d.title ?? d.url}</Link>
                    </td>
                    <td className="py-1 pr-2 mono text-[10px] text-dim">{d.content_hash}</td>
                    <td className={`py-1 pr-2 mono text-[10px] ${EMB_TONE[d.embedding_status] ?? "text-dim"}`}>{en("emb", d.embedding_status)}</td>
                    <td className="py-1"><TimeAgo ts={d.retrieved_at} /></td>
                  </tr>
                ))}
              </tbody>
            </table>
            <div className="mt-2 flex gap-2 text-[11px]">
              <button disabled={offset === 0} onClick={() => setOffset(Math.max(0, offset - limit))}
                className="rounded border border-edge px-2 py-0.5 disabled:opacity-30">{t("common.prev")}</button>
              <button disabled={offset + limit >= total} onClick={() => setOffset(offset + limit)}
                className="rounded border border-edge px-2 py-0.5 disabled:opacity-30">{t("common.next")}</button>
              <span className="ml-auto text-dim">{offset + 1}–{Math.min(offset + limit, total)} / {total}</span>
            </div>
          </>
        )}
      </Panel>
    </div>
  );
}
