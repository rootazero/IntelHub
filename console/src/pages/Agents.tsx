// Agent Workspace (§55/56): per-agent activity timeline, budget state bars
// (§60), cost breakdown (§59), findings count.

import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import { BudgetBadge, Empty, ErrorBox, Loading, Panel, TimeAgo } from "../ui";

interface AgentActivity {
  name: string;
  calls_today: number;
  recent_calls: { tool: string; status: string; latency_ms: number; ts: string }[];
  costs_today: { kind: string; amount: number }[];
  findings_total: number;
  budget: {
    state: string;
    ratio: number;
    usage: { tool_calls: number; crawl_pages: number; embed_tokens: number };
    limits: { tool_calls: number; crawl_pages: number; embed_tokens: number };
  };
}

function BudgetBar({ label, used, limit }: { label: string; used: number; limit: number }) {
  const pct = Math.min(100, (used / Math.max(1, limit)) * 100);
  const tone = pct >= 100 ? "bg-crit" : pct >= 90 ? "bg-crit/70" : pct >= 70 ? "bg-warn" : "bg-ok";
  return (
    <div>
      <div className="flex justify-between text-[10px] text-dim">
        <span>{label}</span>
        <span className="mono">{used.toLocaleString()} / {limit.toLocaleString()}</span>
      </div>
      <div className="mt-0.5 h-1.5 rounded bg-edge">
        <div className={`h-1.5 rounded ${tone}`} style={{ width: `${pct}%` }} />
      </div>
    </div>
  );
}

export default function Agents() {
  const [agents, setAgents] = useState<AgentActivity[]>([]);
  const [error, setError] = useState<Error | null>(null);
  const [loading, setLoading] = useState(true);

  const load = useCallback(() => {
    api<{ agents: AgentActivity[] }>("/api/v1/agents/activity")
      .then((r) => setAgents(r.agents ?? []))
      .catch(setError)
      .finally(() => setLoading(false));
  }, []);
  useEffect(() => {
    load();
    const t = setInterval(load, 15000);
    return () => clearInterval(t);
  }, [load]);

  if (error) return <div className="p-4"><ErrorBox error={error} /></div>;
  if (loading) return <Loading />;

  return (
    <div className="grid grid-cols-1 gap-3 p-4 xl:grid-cols-2">
      {agents.length === 0 && <Empty label="no agents provisioned (hub create-agent)" />}
      {agents.map((a) => (
        <Panel
          key={a.name}
          title={<span className="flex items-center gap-2"><span className="mono text-xs">{a.name}</span><BudgetBadge state={a.budget.state} /></span>}
          right={<span className="text-[10px] text-dim">{a.calls_today} calls today · {a.findings_total} findings total</span>}
        >
          <div className="mb-3 space-y-1.5">
            <BudgetBar label="tool calls" used={a.budget.usage.tool_calls} limit={a.budget.limits.tool_calls} />
            <BudgetBar label="crawl pages" used={a.budget.usage.crawl_pages} limit={a.budget.limits.crawl_pages} />
            <BudgetBar label="embedding tokens" used={a.budget.usage.embed_tokens} limit={a.budget.limits.embed_tokens} />
          </div>
          {a.costs_today.length > 0 && (
            <div className="mb-3 flex flex-wrap gap-2 text-[10px] text-dim">
              {a.costs_today.map((c) => (
                <span key={c.kind} className="rounded border border-edge px-1.5 py-0.5 mono">{c.kind}: {c.amount}</span>
              ))}
            </div>
          )}
          <div className="text-[10px] uppercase tracking-wider text-dim">Recent activity</div>
          {a.recent_calls.length === 0 ? <Empty label="no calls yet" /> : (
            <table className="mt-1 w-full text-[11px]">
              <tbody>
                {a.recent_calls.map((c, i) => (
                  <tr key={i} className="border-t border-edge/50">
                    <td className="py-0.5 mono">{c.tool}</td>
                    <td className={`py-0.5 mono ${c.status === "ok" ? "text-ok" : "text-crit"}`}>{c.status}</td>
                    <td className="py-0.5 mono text-dim">{c.latency_ms}ms</td>
                    <td className="py-0.5 text-right"><TimeAgo ts={c.ts} /></td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Panel>
      ))}
    </div>
  );
}
