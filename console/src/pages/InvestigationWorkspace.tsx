// Investigation Workspace (§27–29): the full aggregate — overview/hypothesis,
// findings with evidence chains (expandable to original documents), entities,
// documents, tasks, alerts, audit. Epistemic types stay visually distinct (§29).

import { useCallback, useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { api } from "../api";
import { useEnum, useT } from "../i18n";
import { Empty, EpistemicBadge, ErrorBox, Loading, Panel, SeverityBadge, TimeAgo } from "../ui";

interface Finding {
  finding_id: string;
  title: string;
  claim_text: string;
  created_by: string;
  created_at: string;
  source_confidence?: number | null;
  claim_confidence?: number | null;
  evidence?: { document_id: string; relation: string; url?: string; title?: string }[];
}

interface Workspace {
  investigation: {
    investigation_id: string; title: string; question?: string | null;
    target?: string | null; hypothesis?: string | null; status: string;
    created_by: string; created_at: string;
  };
  findings: { items?: Finding[] } | Finding[];
  documents: { document_id: string; url: string; title?: string | null; retrieved_at: string }[];
  entities: { entity_id: string; kind: string; name: string }[];
  tasks: { task_id: string; kind: string; status: string; created_by: string; created_at: string }[];
  alerts: { alert_id: string; severity: string; source: string; title: string; status: string }[];
  audit: { actor: string; action: string; result: string; ts: string }[];
}

function FindingCard({ f }: { f: Finding }) {
  const { t } = useT();
  const en = useEnum();
  const [open, setOpen] = useState(false);
  const evidence = f.evidence ?? [];
  const supporting = evidence.filter((e) => e.relation === "supports").length;
  const contradicting = evidence.filter((e) => e.relation === "contradicts").length;
  return (
    <div className="rounded border border-edge bg-base p-2">
      <div className="flex items-start justify-between gap-2">
        <div>
          <div className="flex items-center gap-2">
            <EpistemicBadge kind="AGENT CLAIM" />
            <span className="text-xs font-semibold">{f.title}</span>
          </div>
          <p className="mt-1 text-xs text-ink/80">{f.claim_text}</p>
        </div>
        <div className="shrink-0 text-right text-[10px] text-dim mono">
          <div>{f.created_by}</div>
          <TimeAgo ts={f.created_at} />
        </div>
      </div>
      <div className="mt-1.5 flex items-center gap-3 text-[10px] text-dim">
        <span>{t("workspace.supporting")} <b className="text-ok">{supporting}</b></span>
        <span>{t("workspace.contradicting")} <b className="text-crit">{contradicting}</b></span>
        {f.source_confidence != null && <span>{t("workspace.sourceConf")} {f.source_confidence}</span>}
        {f.claim_confidence != null && <span>{t("workspace.claimConf")} {f.claim_confidence}</span>}
        <button onClick={() => setOpen(!open)} className="ml-auto text-accent hover:underline">
          {open ? t("workspace.hideEvidence") : t("workspace.viewEvidence", { n: evidence.length })}
        </button>
      </div>
      {open && (
        <ul className="mt-1.5 space-y-1 border-t border-edge pt-1.5">
          {evidence.map((e) => (
            <li key={e.document_id + e.relation} className="flex items-center gap-2 text-[11px]">
              <EpistemicBadge kind="FACT" />
              <span className={`mono ${e.relation === "contradicts" ? "text-crit" : "text-ok"}`}>{en("rel", e.relation)}</span>
              <Link className="truncate text-accent hover:underline" to={`/evidence/${e.document_id}`}>
                {e.title ?? e.url ?? e.document_id}
              </Link>
            </li>
          ))}
          {evidence.length === 0 && <li className="text-[11px] text-dim">{t("workspace.noLinkedEvidence")}</li>}
        </ul>
      )}
    </div>
  );
}

export default function InvestigationWorkspace() {
  const { t } = useT();
  const { id } = useParams();
  const [ws, setWs] = useState<Workspace | null>(null);
  const [error, setError] = useState<Error | null>(null);

  const load = useCallback(() => {
    if (!id) return;
    api<Workspace>(`/api/v1/investigations/${id}/workspace`).then(setWs).catch(setError);
  }, [id]);
  useEffect(load, [load]);

  if (error) return <div className="p-4"><ErrorBox error={error} /></div>;
  if (!ws) return <Loading />;
  const inv = ws.investigation;
  const findings = Array.isArray(ws.findings) ? ws.findings : ws.findings.items ?? [];

  return (
    <div className="space-y-3 p-4">
      <div className="rounded border border-edge bg-panel p-3">
        <div className="flex items-center gap-2">
          <EpistemicBadge kind={inv.hypothesis ? "HYPOTHESIS" : "SYSTEM STATE"} />
          <h1 className="text-sm font-bold">{inv.title}</h1>
          <span className={`mono text-[10px] ${inv.status === "open" ? "text-ok" : "text-dim"}`}>{inv.status}</span>
        </div>
        <div className="mt-1 grid grid-cols-1 gap-1 text-xs md:grid-cols-3">
          <div><span className="text-dim">{t("workspace.target")}</span><span className="mono">{inv.target ?? "—"}</span></div>
          <div className="md:col-span-2"><span className="text-dim">{t("workspace.question")}</span>{inv.question ?? "—"}</div>
        </div>
        {inv.hypothesis && <p className="mt-1 text-xs text-purple-300/90">{t("workspace.hypothesis")}{inv.hypothesis}</p>}
      </div>

      <div className="grid grid-cols-1 gap-3 xl:grid-cols-2">
        <Panel title={t("workspace.findings", { n: findings.length })}>
          {findings.length === 0 ? <Empty label={t("workspace.noFindings")} /> : (
            <div className="space-y-2">{findings.map((f) => <FindingCard key={f.finding_id} f={f} />)}</div>
          )}
        </Panel>

        <div className="space-y-3">
          <Panel title={t("workspace.documents", { n: ws.documents.length })}>
            {ws.documents.length === 0 ? <Empty label={t("workspace.noDocs")} /> : (
              <ul className="max-h-64 space-y-1 overflow-y-auto text-xs">
                {ws.documents.map((d) => (
                  <li key={d.document_id} className="flex items-center gap-2">
                    <EpistemicBadge kind="FACT" />
                    <Link className="truncate text-accent hover:underline" to={`/evidence/${d.document_id}`}>
                      {d.title ?? d.url}
                    </Link>
                    <TimeAgo ts={d.retrieved_at} />
                  </li>
                ))}
              </ul>
            )}
          </Panel>

          <Panel title={t("workspace.entities", { n: ws.entities.length })}>
            {ws.entities.length === 0 ? <Empty label={t("workspace.noEntities")} /> : (
              <div className="flex flex-wrap gap-1.5">
                {ws.entities.map((e) => (
                  <Link key={e.entity_id} to={`/entities/${e.entity_id}`}
                    className="rounded border border-edge bg-base px-2 py-0.5 text-[11px] hover:border-accent">
                    <span className="text-dim">{e.kind}:</span>{e.name}
                  </Link>
                ))}
              </div>
            )}
          </Panel>

          <Panel title={t("workspace.alerts", { n: ws.alerts.length })}>
            {ws.alerts.length === 0 ? <Empty label={t("workspace.noAlerts")} /> : (
              <ul className="space-y-1 text-xs">
                {ws.alerts.map((a) => (
                  <li key={a.alert_id} className="flex items-center gap-2">
                    <SeverityBadge severity={a.severity} />
                    <span className="flex-1 truncate">{a.title}</span>
                    <span className="mono text-[10px] text-dim">{a.status}</span>
                  </li>
                ))}
              </ul>
            )}
          </Panel>
        </div>
      </div>

      <div className="grid grid-cols-1 gap-3 xl:grid-cols-2">
        <Panel title={t("workspace.tasks", { n: ws.tasks.length })}>
          {ws.tasks.length === 0 ? <Empty label={t("workspace.noTasks")} /> : (
            <table className="w-full text-xs">
              <tbody>
                {ws.tasks.map((t) => (
                  <tr key={t.task_id} className="border-t border-edge/50">
                    <td className="py-1 mono">{t.kind}</td>
                    <td className={`py-1 mono ${t.status === "completed" ? "text-ok" : t.status === "failed" ? "text-crit" : "text-warn"}`}>{t.status}</td>
                    <td className="py-1 mono text-dim">{t.created_by}</td>
                    <td className="py-1 text-right"><TimeAgo ts={t.created_at} /></td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Panel>

        <Panel title={t("workspace.audit", { n: ws.audit.length })}>
          {ws.audit.length === 0 ? <Empty label={t("workspace.noAudit")} /> : (
            <ul className="space-y-0.5 mono text-[11px]">
              {ws.audit.map((a, i) => (
                <li key={i} className="flex gap-2">
                  <TimeAgo ts={a.ts} />
                  <span className="text-dim">{a.actor}</span>
                  <span>{a.action}</span>
                  <span className={a.result === "ok" ? "text-ok" : "text-crit"}>{a.result}</span>
                </li>
              ))}
            </ul>
          )}
        </Panel>
      </div>
    </div>
  );
}
