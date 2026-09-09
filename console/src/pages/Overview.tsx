// Overview (§25): system health, active investigations, recent alerts, sensor
// activity, agent activity, evidence ingestion, graph changes, semantic
// memory, cloud API usage + live event stream (§63). Radar placeholder → SP4.

import { useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { api, streamEvents, type BusEventPayload } from "../api";
import { BudgetBadge, Empty, ErrorBox, Panel, SeverityBadge, Stat, StatusDot, TimeAgo } from "../ui";

interface OverviewData {
  ts: string;
  health: { components: Record<string, { status: string; latency_ms: number }> };
  investigations: { active: number; items: { items?: { investigation_id: string; title: string; status: string }[] } };
  alerts: { open: number; recent: { items?: { alert_id: string; severity: string; title: string; created_at: string }[] } };
  agents: { name: string; calls_today: number; budget_state: string; budget_ratio: number }[];
  evidence: { docs_today: number; docs_total: number };
  graph: { entities: number; relationships: number; changes_today: number };
  memory: { embedded_docs: number; chunks: number };
  cloud: { embedding_tokens_today: number; est_cost_usd: number };
  radar?: {
    geo_events_24h: number;
    crucix: { up: boolean; sources_ok?: number | null; sources_failed?: number | null; last_sweep?: string | null };
  };
}

export default function Overview() {
  const [data, setData] = useState<OverviewData | null>(null);
  const [error, setError] = useState<Error | null>(null);
  const [events, setEvents] = useState<BusEventPayload[]>([]);
  const [streamOk, setStreamOk] = useState(true);
  const seen = useRef(new Set<string>());

  useEffect(() => {
    api<OverviewData>("/api/v1/overview").then(setData).catch(setError);
    const stop = streamEvents(
      (ev) => {
        if (seen.current.has(ev.event_id)) return;
        seen.current.add(ev.event_id);
        setStreamOk(true);
        setEvents((prev) => [ev, ...prev].slice(0, 40));
      },
      () => setStreamOk(false),
    );
    const t = setInterval(() => api<OverviewData>("/api/v1/overview").then(setData).catch(() => {}), 30000);
    return () => {
      stop();
      clearInterval(t);
    };
  }, []);

  if (error) return <div className="p-4"><ErrorBox error={error} /></div>;
  if (!data) return <div className="p-4 text-dim mono text-xs">loading…</div>;

  const comps = Object.entries(data.health.components);
  const upCount = comps.filter(([, c]) => c.status === "up").length;

  return (
    <div className="p-4 space-y-3">
      <div className="grid grid-cols-2 gap-2 md:grid-cols-4 xl:grid-cols-8">
        <Stat label="Components up" value={`${upCount}/${comps.length}`} tone={upCount === comps.length ? "ok" : "crit"} />
        <Stat label="Active investigations" value={data.investigations.active} tone={data.investigations.active > 0 ? "ok" : "dim"} />
        <Stat label="Open alerts" value={data.alerts.open} tone={data.alerts.open > 0 ? "warn" : "ok"} />
        <Stat label="Evidence today" value={data.evidence.docs_today} />
        <Stat label="Evidence total" value={data.evidence.docs_total} />
        <Stat label="Graph changes today" value={data.graph.changes_today} />
        <Stat label="Embedded docs" value={data.memory.embedded_docs} />
        <Stat label="Cloud cost today" value={`$${data.cloud.est_cost_usd}`} tone={data.cloud.est_cost_usd > 1 ? "warn" : "dim"} />
      </div>

      <div className="grid grid-cols-1 gap-3 xl:grid-cols-3">
        <Panel title="System Health" right={<Link to="/system" className="text-[10px] text-accent">details →</Link>}>
          <div className="grid grid-cols-2 gap-1.5">
            {comps.map(([name, c]) => (
              <div key={name} className="flex items-center justify-between rounded border border-edge px-2 py-1">
                <span className="flex items-center gap-1.5 text-xs"><StatusDot up={c.status === "up"} />{name}</span>
                <span className="mono text-[10px] text-dim">{c.latency_ms}ms</span>
              </div>
            ))}
          </div>
        </Panel>

        <Panel title="Agents" right={<Link to="/agents" className="text-[10px] text-accent">activity →</Link>}>
          {data.agents.length === 0 ? <Empty label="no agents" /> : (
            <table className="w-full text-xs">
              <thead><tr className="text-left text-[10px] uppercase text-dim">
                <th className="pb-1">Agent</th><th className="pb-1 text-right">Calls today</th><th className="pb-1 text-right">Budget</th>
              </tr></thead>
              <tbody>
                {data.agents.map((a) => (
                  <tr key={a.name} className="border-t border-edge/50">
                    <td className="py-1 mono">{a.name}</td>
                    <td className="py-1 text-right mono">{a.calls_today}</td>
                    <td className="py-1 text-right"><BudgetBadge state={a.budget_state} /></td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Panel>

        <Panel title="Global Intelligence Radar" className="min-h-32" right={<Link to="/radar" className="text-[10px] text-accent">map →</Link>}>
          <div className="flex h-full flex-col justify-center gap-2">
            <div className="flex items-baseline gap-2">
              <span className="text-2xl font-bold mono">{data.radar?.geo_events_24h ?? 0}</span>
              <span className="text-[10px] uppercase tracking-wider text-dim">geo events / 24h</span>
            </div>
            <div className="flex items-center gap-2 text-xs">
              <StatusDot up={data.radar?.crucix.up ?? false} />
              <span className="text-dim">Crucix</span>
              {data.radar?.crucix.up && (
                <span className="mono text-[10px] text-dim">
                  {data.radar.crucix.sources_ok ?? "?"} sources ok
                  {(data.radar.crucix.sources_failed ?? 0) > 0 && (
                    <span className="text-amber-400"> · {data.radar.crucix.sources_failed} failed</span>
                  )}
                </span>
              )}
            </div>
            {data.radar?.crucix.last_sweep && (
              <div className="text-[10px] text-dim">
                last sweep <TimeAgo ts={data.radar.crucix.last_sweep} />
              </div>
            )}
          </div>
        </Panel>
      </div>

      <div className="grid grid-cols-1 gap-3 xl:grid-cols-3">
        <Panel title="Recent Alerts" right={<Link to="/alerts" className="text-[10px] text-accent">center →</Link>}>
          {(data.alerts.recent.items ?? []).length === 0 ? <Empty label="no open alerts" /> : (
            <ul className="space-y-1">
              {(data.alerts.recent.items ?? []).map((a) => (
                <li key={a.alert_id} className="flex items-center gap-2 text-xs">
                  <SeverityBadge severity={a.severity} />
                  <span className="flex-1 truncate">{a.title}</span>
                  <TimeAgo ts={a.created_at} />
                </li>
              ))}
            </ul>
          )}
        </Panel>

        <Panel title="Active Investigations" right={<Link to="/investigations" className="text-[10px] text-accent">all →</Link>}>
          {(data.investigations.items.items ?? []).length === 0 ? <Empty label="no open investigations" /> : (
            <ul className="space-y-1">
              {(data.investigations.items.items ?? []).map((i) => (
                <li key={i.investigation_id} className="text-xs">
                  <Link className="text-accent hover:underline" to={`/investigations/${i.investigation_id}`}>{i.title}</Link>
                </li>
              ))}
            </ul>
          )}
        </Panel>

        <Panel
          title="Live Event Stream"
          right={<span className="flex items-center gap-1.5 text-[10px] text-dim"><StatusDot up={streamOk} />SSE</span>}
        >
          {events.length === 0 ? <Empty label="waiting for events…" /> : (
            <ul className="max-h-64 space-y-1 overflow-y-auto mono text-[11px]">
              {events.map((e) => (
                <li key={e.event_id} className="flex gap-2">
                  <span className="shrink-0 text-accent">{e.event_type}</span>
                  <span className="shrink-0 text-dim">{e.actor}</span>
                  <TimeAgo ts={e.ts} />
                </li>
              ))}
            </ul>
          )}
        </Panel>
      </div>
    </div>
  );
}
