// Audit Center (§66): who / what / when / why / source / result — filterable.

import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import { Empty, ErrorBox, Loading, Panel, TimeAgo } from "../ui";

interface AuditRow {
  audit_id: string;
  actor: string;
  action: string;
  object_type?: string | null;
  object_id?: string | null;
  result: string;
  ts: string;
}

export default function Audit() {
  const [items, setItems] = useState<AuditRow[]>([]);
  const [error, setError] = useState<Error | null>(null);
  const [loading, setLoading] = useState(true);
  const [actor, setActor] = useState("");
  const [action, setAction] = useState("");
  const [result, setResult] = useState("");

  const load = useCallback(() => {
    const params = new URLSearchParams();
    if (actor) params.set("actor", actor);
    if (action) params.set("action", action);
    if (result) params.set("result", result);
    params.set("limit", "200");
    api<{ items: AuditRow[] }>(`/api/v1/audit?${params}`)
      .then((r) => setItems(r.items ?? []))
      .catch(setError)
      .finally(() => setLoading(false));
  }, [actor, action, result]);

  useEffect(load, [load]);

  return (
    <div className="p-4">
      <Panel
        title="Audit Center"
        right={
          <div className="flex gap-1.5 text-[11px]">
            <input value={actor} onChange={(e) => setActor(e.target.value)} placeholder="actor (e.g. agent:codex)"
              className="w-44 rounded border border-edge bg-base px-1.5 py-0.5 mono" />
            <input value={action} onChange={(e) => setAction(e.target.value)} placeholder="action"
              className="w-32 rounded border border-edge bg-base px-1.5 py-0.5 mono" />
            <select value={result} onChange={(e) => setResult(e.target.value)} className="rounded border border-edge bg-base px-1.5 py-0.5">
              <option value="">any result</option><option value="ok">ok</option><option value="error">error</option><option value="denied">denied</option>
            </select>
          </div>
        }
      >
        {error && <ErrorBox error={error} />}
        {loading ? <Loading /> : items.length === 0 ? <Empty label="no audit records match" /> : (
          <table className="w-full text-xs">
            <thead><tr className="text-left text-[10px] uppercase text-dim">
              <th className="pb-1">When</th><th className="pb-1">Who</th><th className="pb-1">Action</th>
              <th className="pb-1">Object</th><th className="pb-1">Result</th>
            </tr></thead>
            <tbody>
              {items.map((a) => (
                <tr key={a.audit_id} className="border-t border-edge/50">
                  <td className="py-1 pr-2"><TimeAgo ts={a.ts} /></td>
                  <td className="py-1 pr-2 mono">{a.actor}</td>
                  <td className="py-1 pr-2 mono">{a.action}</td>
                  <td className="py-1 pr-2 mono text-dim">
                    {a.object_type ? `${a.object_type}:${(a.object_id ?? "").slice(0, 18)}` : "—"}
                  </td>
                  <td className={`py-1 mono ${a.result === "ok" ? "text-ok" : a.result === "denied" ? "text-warn" : "text-crit"}`}>
                    {a.result}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Panel>
    </div>
  );
}
