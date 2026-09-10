// Unified Search (§67): one box — the system picks keyword/metadata/graph/
// semantic/hybrid; the user never chooses a database.

import { useEffect, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { api } from "../api";
import { useT } from "../i18n";
import { Empty, ErrorBox, Panel } from "../ui";

interface UnifiedResult {
  query: string;
  entities: { entity_id: string; kind: string; name: string; created_by: string }[];
  documents: { items?: { document_id: string; url: string; title?: string | null; rank?: number }[] };
  similar_documents: { document_id: string; score: number; url: string; title?: string | null }[];
  relationships: { from: string; rel_type: string; to: string }[];
  findings: { finding_id: string; investigation_id: string; title: string; claim_text: string; created_by: string }[];
  alerts: { alert_id: string; severity: string; source: string; title: string; status: string }[];
  investigations: { investigation_id: string; title: string; target?: string | null; status: string }[];
}

export default function SearchPage() {
  const { t } = useT();
  const [params, setParams] = useSearchParams();
  const [q, setQ] = useState(params.get("q") ?? "");
  const [result, setResult] = useState<UnifiedResult | null>(null);
  const [error, setError] = useState<Error | null>(null);
  const [busy, setBusy] = useState(false);

  const run = (query: string) => {
    if (!query.trim()) return;
    setBusy(true);
    setError(null);
    api<UnifiedResult>(`/api/v1/search/unified?q=${encodeURIComponent(query.trim())}`)
      .then(setResult)
      .catch(setError)
      .finally(() => setBusy(false));
  };

  useEffect(() => {
    const initial = params.get("q");
    if (initial) run(initial);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="space-y-3 p-4">
      <form
        onSubmit={(e) => {
          e.preventDefault();
          setParams({ q });
          run(q);
        }}
        className="flex gap-2"
      >
        <input
          autoFocus
          value={q}
          onChange={(e) => setQ(e.target.value)}
          placeholder={t("search.placeholder")}
          className="flex-1 rounded border border-edge bg-panel px-3 py-2 text-sm outline-none focus:border-accent"
        />
        <button className="rounded bg-accent/20 px-4 py-2 text-sm font-semibold text-accent hover:bg-accent/30">
          {busy ? "…" : t("search.button")}
        </button>
      </form>

      {error && <ErrorBox error={error} />}
      {result && (
        <div className="grid grid-cols-1 gap-3 xl:grid-cols-2">
          <Panel title={t("search.entities", { n: result.entities.length })}>
            {result.entities.length === 0 ? <Empty label={t("search.noEntity")} /> : (
              <div className="flex flex-wrap gap-1.5">
                {result.entities.map((e) => (
                  <Link key={e.entity_id} to={`/entities/${e.entity_id}`}
                    className="rounded border border-edge bg-base px-2 py-0.5 text-[11px] hover:border-accent">
                    <span className="text-dim">{e.kind}:</span>{e.name}
                  </Link>
                ))}
              </div>
            )}
          </Panel>

          <Panel title={t("search.relationships", { n: result.relationships.length })}>
            {result.relationships.length === 0 ? <Empty label={t("search.noRel")} /> : (
              <ul className="space-y-0.5 mono text-[11px]">
                {result.relationships.map((r, i) => (
                  <li key={i}>{r.from} <span className="text-accent">—{r.rel_type}→</span> {r.to}</li>
                ))}
              </ul>
            )}
          </Panel>

          <Panel title={t("search.docsKeyword", { n: result.documents.items?.length ?? 0 })}>
            {(result.documents.items ?? []).length === 0 ? <Empty label={t("search.noKeyword")} /> : (
              <ul className="space-y-1 text-xs">
                {(result.documents.items ?? []).map((d) => (
                  <li key={d.document_id}>
                    <Link className="text-accent hover:underline" to={`/evidence/${d.document_id}`}>{d.title ?? d.url}</Link>
                  </li>
                ))}
              </ul>
            )}
          </Panel>

          <Panel title={t("search.docsSemantic", { n: result.similar_documents.length })}>
            {result.similar_documents.length === 0 ? <Empty label={t("search.noSemantic")} /> : (
              <ul className="space-y-1 text-xs">
                {result.similar_documents.map((d) => (
                  <li key={d.document_id} className="flex items-center gap-2">
                    <span className="mono text-[10px] text-dim">{d.score.toFixed(3)}</span>
                    <Link className="truncate text-accent hover:underline" to={`/evidence/${d.document_id}`}>{d.title ?? d.url}</Link>
                  </li>
                ))}
              </ul>
            )}
          </Panel>

          <Panel title={t("search.findings", { n: result.findings.length })}>
            {result.findings.length === 0 ? <Empty label={t("search.noFinding")} /> : (
              <ul className="space-y-1.5 text-xs">
                {result.findings.map((f) => (
                  <li key={f.finding_id}>
                    <Link className="text-accent hover:underline" to={`/investigations/${f.investigation_id}`}>{f.title}</Link>
                    <span className="ml-2 mono text-[10px] text-dim">{f.created_by}</span>
                  </li>
                ))}
              </ul>
            )}
          </Panel>

          <Panel title={t("search.invAlerts", { n: result.investigations.length, m: result.alerts.length })}>
            <ul className="space-y-1 text-xs">
              {result.investigations.map((i) => (
                <li key={i.investigation_id}>
                  <Link className="text-accent hover:underline" to={`/investigations/${i.investigation_id}`}>{i.title}</Link>
                  <span className="ml-2 mono text-[10px] text-dim">{i.status}</span>
                </li>
              ))}
              {result.alerts.map((a) => (
                <li key={a.alert_id} className="text-dim">[{a.severity}] {a.title}</li>
              ))}
              {result.investigations.length === 0 && result.alerts.length === 0 && <Empty label={t("search.noMatch")} />}
            </ul>
          </Panel>
        </div>
      )}
    </div>
  );
}
