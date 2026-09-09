// Document detail (§43/§29): provenance chain + reverse references
// (which findings/claims cite this evidence, which entities were observed).

import { useCallback, useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { api } from "../api";
import { Empty, EpistemicBadge, ErrorBox, Loading, Panel, TimeAgo } from "../ui";

interface Detail {
  document: {
    document_id: string; url: string; title?: string | null;
    content_text: string; content_hash: string; retrieved_at: string;
    published_at?: string | null; embedding_status: string;
    provenance?: Record<string, unknown>; metadata?: Record<string, unknown>;
  };
  referenced_by_findings: { finding_id: string; investigation_id: string; title: string; relation: string; created_by: string }[];
  referenced_by_claims: { claim_id: string; text: string; status: string; relation: string }[];
  observed_entities: { entity_id: string; kind: string; name: string }[];
}

export default function DocumentDetail() {
  const { id } = useParams();
  const [detail, setDetail] = useState<Detail | null>(null);
  const [error, setError] = useState<Error | null>(null);
  const [showFull, setShowFull] = useState(false);

  const load = useCallback(() => {
    if (!id) return;
    api<Detail>(`/api/v1/documents/${id}`).then(setDetail).catch(setError);
  }, [id]);
  useEffect(load, [load]);

  if (error) return <div className="p-4"><ErrorBox error={error} /></div>;
  if (!detail) return <Loading />;
  const d = detail.document;

  return (
    <div className="space-y-3 p-4">
      <div className="rounded border border-edge bg-panel p-3">
        <div className="flex items-center gap-2">
          <EpistemicBadge kind="FACT" />
          <h1 className="text-sm font-bold">{d.title ?? d.url}</h1>
        </div>
        <div className="mt-1 break-all mono text-[11px] text-dim">
          <a className="text-accent hover:underline" href={d.url} target="_blank" rel="noreferrer">{d.url}</a>
        </div>
        <div className="mt-2 grid grid-cols-2 gap-1 text-[11px] md:grid-cols-4">
          <div><span className="text-dim">retrieved: </span><TimeAgo ts={d.retrieved_at} /></div>
          <div><span className="text-dim">published: </span>{d.published_at ? <TimeAgo ts={d.published_at} /> : "—"}</div>
          <div><span className="text-dim">hash: </span><span className="mono">{d.content_hash.slice(0, 16)}…</span></div>
          <div><span className="text-dim">embedding: </span><span className="mono">{d.embedding_status}</span></div>
        </div>
      </div>

      <div className="grid grid-cols-1 gap-3 xl:grid-cols-2">
        <Panel title="Content">
          <div className="whitespace-pre-wrap text-xs leading-relaxed text-ink/80">
            {showFull ? d.content_text : d.content_text.slice(0, 3000)}
            {d.content_text.length > 3000 && (
              <button onClick={() => setShowFull(!showFull)} className="ml-2 text-accent hover:underline">
                {showFull ? "collapse ▲" : `show all (${d.content_text.length} chars) ▼`}
              </button>
            )}
          </div>
        </Panel>

        <div className="space-y-3">
          <Panel title={`Referenced by Findings (${detail.referenced_by_findings.length})`}>
            {detail.referenced_by_findings.length === 0 ? <Empty label="no findings cite this evidence" /> : (
              <ul className="space-y-1 text-xs">
                {detail.referenced_by_findings.map((f) => (
                  <li key={f.finding_id} className="flex items-center gap-2">
                    <EpistemicBadge kind="AGENT CLAIM" />
                    <span className={f.relation === "contradicts" ? "text-crit" : "text-ok"}>{f.relation}</span>
                    <Link className="truncate text-accent hover:underline" to={`/investigations/${f.investigation_id}`}>{f.title}</Link>
                    <span className="mono text-[10px] text-dim">{f.created_by}</span>
                  </li>
                ))}
              </ul>
            )}
          </Panel>

          <Panel title={`Referenced by Claims (${detail.referenced_by_claims.length})`}>
            {detail.referenced_by_claims.length === 0 ? <Empty label="no claims cite this evidence" /> : (
              <ul className="space-y-1 text-xs">
                {detail.referenced_by_claims.map((c) => (
                  <li key={c.claim_id} className="flex items-start gap-2">
                    <EpistemicBadge kind="AGENT CLAIM" />
                    <span className="flex-1">{c.text}</span>
                    <span className="mono text-[10px] text-dim">{c.status}</span>
                  </li>
                ))}
              </ul>
            )}
          </Panel>

          <Panel title={`Observed Entities (${detail.observed_entities.length})`}>
            {detail.observed_entities.length === 0 ? <Empty label="no entities observed on this document" /> : (
              <div className="flex flex-wrap gap-1.5">
                {detail.observed_entities.map((e) => (
                  <Link key={e.entity_id} to={`/entities/${e.entity_id}`}
                    className="rounded border border-edge bg-base px-2 py-0.5 text-[11px] hover:border-accent">
                    <span className="text-dim">{e.kind}:</span>{e.name}
                  </Link>
                ))}
              </div>
            )}
          </Panel>
        </div>
      </div>
    </div>
  );
}
