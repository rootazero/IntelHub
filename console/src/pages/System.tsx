// System: Health Center (§68) + Component Manager (§69) + sensor status.
// L3 actions (backup/upgrade/rollback) require the admin token per-use —
// it is held in memory only, never persisted (policy engine §61 default DENY).

import { useCallback, useEffect, useState } from "react";
import { api, ApiError } from "../api";
import { Empty, ErrorBox, Loading, Panel, StatusDot } from "../ui";

interface Health {
  version: string;
  components: Record<string, { status: string; latency_ms: number }>;
  containers: { Names: string; Image: string; Status: string; State: string }[] | { error: string };
}

interface Component {
  name: string;
  container: string;
  state: string;
  status: string;
  installed_version?: string | null;
  latest_known_version?: string | null;
  update_available: boolean;
  note?: string;
}

export default function System() {
  const [health, setHealth] = useState<Health | null>(null);
  const [components, setComponents] = useState<Component[]>([]);
  const [error, setError] = useState<Error | null>(null);
  const [actionLog, setActionLog] = useState<string>("");

  const load = useCallback(() => {
    api<Health>("/api/v1/health").then(setHealth).catch(setError);
    api<{ items: Component[] }>("/api/v1/components").then((r) => setComponents(r.items ?? [])).catch(setError);
  }, []);
  useEffect(() => {
    load();
    const t = setInterval(load, 30000);
    return () => clearInterval(t);
  }, [load]);

  const runAction = async (component: string, action: string) => {
    const token = window.prompt(`Level 3 action — enter admin token to ${action} ${component}:`);
    if (!token) return;
    setActionLog(`running ${action} on ${component}…`);
    try {
      const key = localStorage.getItem("intelhub.console.key") ?? "";
      const resp = await fetch(`/api/v1/components/${component}/actions`, {
        method: "POST",
        headers: { Authorization: `Bearer ${key}`, "X-Admin-Token": token, "Content-Type": "application/json" },
        body: JSON.stringify({ action }),
      });
      const body = await resp.json();
      if (!resp.ok) throw new ApiError(resp.status, body.error ?? "failed");
      setActionLog(`✓ ${action} ${component} ok`);
      load();
    } catch (e) {
      setActionLog(`✗ ${e instanceof Error ? e.message : String(e)}`);
    }
  };

  return (
    <div className="space-y-3 p-4">
      {error && <ErrorBox error={error} />}

      <Panel title={`Health Center — hub-core v${health?.version ?? "…"}`}>
        {!health ? <Loading /> : (
          <div className="grid grid-cols-2 gap-1.5 md:grid-cols-3 xl:grid-cols-6">
            {Object.entries(health.components).map(([name, c]) => (
              <div key={name} className="rounded border border-edge px-2 py-1.5">
                <div className="flex items-center gap-1.5 text-xs font-semibold">
                  <StatusDot up={c.status === "up"} />{name}
                </div>
                <div className="mt-0.5 mono text-[10px] text-dim">{c.status} · {c.latency_ms}ms</div>
              </div>
            ))}
          </div>
        )}
      </Panel>

      <Panel
        title="Component Manager"
        right={<span className="text-[10px] text-dim">L3 actions prompt for admin token · default DENY (§61)</span>}
      >
        {components.length === 0 ? <Empty label="no component data" /> : (
          <table className="w-full text-xs">
            <thead><tr className="text-left text-[10px] uppercase text-dim">
              <th className="pb-1">Component</th><th className="pb-1">State</th><th className="pb-1">Installed</th>
              <th className="pb-1">Latest known</th><th className="pb-1">Update</th><th className="pb-1 text-right">Actions</th>
            </tr></thead>
            <tbody>
              {components.map((c) => (
                <tr key={c.name} className="border-t border-edge/50">
                  <td className="py-1.5 mono font-semibold">{c.name}</td>
                  <td className="py-1.5">
                    <span className="flex items-center gap-1.5">
                      <StatusDot up={c.state === "running"} />
                      <span className="text-dim">{c.status || c.state}</span>
                    </span>
                  </td>
                  <td className="py-1.5 mono">{c.installed_version ?? c.note ?? "—"}</td>
                  <td className="py-1.5 mono">{c.latest_known_version ?? "—"}</td>
                  <td className="py-1.5">
                    {c.update_available
                      ? <span className="rounded border border-warn/40 px-1.5 py-0 text-[10px] font-bold text-warn">UPDATE</span>
                      : <span className="text-[10px] text-dim">current</span>}
                  </td>
                  <td className="py-1.5 text-right whitespace-nowrap">
                    {["backup", "upgrade", "rollback"].map((a) => (
                      <button
                        key={a}
                        onClick={() => runAction(c.name, a)}
                        className="ml-1 rounded border border-edge px-1.5 py-0.5 text-[10px] text-dim hover:border-accent hover:text-accent"
                      >
                        {a}
                      </button>
                    ))}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        {actionLog && <div className="mt-2 rounded border border-edge bg-base p-2 mono text-[11px] text-dim">{actionLog}</div>}
      </Panel>

      <Panel title="Containers (docker ps)">
        {!health ? <Loading /> : Array.isArray(health.containers) ? (
          <div className="grid grid-cols-1 gap-1 md:grid-cols-2">
            {health.containers.map((c) => (
              <div key={c.Names} className="flex items-center justify-between rounded border border-edge px-2 py-1 text-[11px]">
                <span className="flex items-center gap-1.5"><StatusDot up={c.State === "running"} /><span className="mono">{c.Names}</span></span>
                <span className="mono text-[10px] text-dim">{c.Image}</span>
              </div>
            ))}
          </div>
        ) : <Empty label="docker telemetry unavailable" />}
      </Panel>
    </div>
  );
}
