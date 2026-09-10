// Alert Center (§54): unified alerts with severity/source filters,
// acknowledge / mute / investigate actions, recommended actions.

import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { api } from "../api";
import { useEnum, useT } from "../i18n";
import { Empty, ErrorBox, Loading, Panel, SeverityBadge, TimeAgo } from "../ui";

interface Alert {
  alert_id: string;
  severity: string;
  source: string;
  title: string;
  recommended_action?: string | null;
  status: string;
  occurrence: number;
  investigation_id?: string | null;
  created_at: string;
}

export default function Alerts() {
  const { t } = useT();
  const en = useEnum();
  const [items, setItems] = useState<Alert[]>([]);
  const [error, setError] = useState<Error | null>(null);
  const [loading, setLoading] = useState(true);
  const [status, setStatus] = useState("open");
  const [source, setSource] = useState("");
  const [severity, setSeverity] = useState("");
  const nav = useNavigate();

  const load = useCallback(() => {
    const params = new URLSearchParams();
    if (status) params.set("status", status);
    if (source) params.set("source", source);
    if (severity) params.set("severity", severity);
    params.set("limit", "100");
    api<{ items: Alert[] }>(`/api/v1/alerts?${params}`)
      .then((r) => setItems(r.items ?? []))
      .catch(setError)
      .finally(() => setLoading(false));
  }, [status, source, severity]);

  useEffect(load, [load]);

  const act = async (id: string, action: "ack" | "mute") => {
    try {
      await api(`/api/v1/alerts/${id}/${action}`, { method: "POST" });
      load();
    } catch (e) {
      setError(e instanceof Error ? e : new Error(String(e)));
    }
  };

  return (
    <div className="p-4">
      <Panel
        title={t("alerts.title")}
        right={
          <div className="flex gap-1.5 text-[11px]">
            <select value={status} onChange={(e) => setStatus(e.target.value)} className="rounded border border-edge bg-base px-1.5 py-0.5">
              <option value="open">{en("alertStatus", "open")}</option><option value="ack">{en("alertStatus", "ack")}</option><option value="muted">{en("alertStatus", "muted")}</option><option value="">{t("common.all")}</option>
            </select>
            <select value={source} onChange={(e) => setSource(e.target.value)} className="rounded border border-edge bg-base px-1.5 py-0.5">
              <option value="">{t("alerts.anySource")}</option>
              {["osint", "agent", "sensor", "infra", "budget", "security"].map((s) => <option key={s} value={s}>{en("alertSource", s)}</option>)}
            </select>
            <select value={severity} onChange={(e) => setSeverity(e.target.value)} className="rounded border border-edge bg-base px-1.5 py-0.5">
              <option value="">{t("alerts.anySeverity")}</option>
              {["critical", "warning", "info"].map((s) => <option key={s} value={s}>{en("severity", s)}</option>)}
            </select>
          </div>
        }
      >
        {error && <ErrorBox error={error} />}
        {loading ? <Loading /> : items.length === 0 ? <Empty label={t("alerts.noMatch")} /> : (
          <table className="w-full text-xs">
            <thead><tr className="text-left text-[10px] uppercase text-dim">
              <th className="pb-1">{t("alerts.thSeverity")}</th><th className="pb-1">{t("alerts.thSource")}</th><th className="pb-1">{t("alerts.thAlert")}</th>
              <th className="pb-1">{t("alerts.thOcc")}</th><th className="pb-1">{t("common.age")}</th><th className="pb-1 text-right">{t("common.actions")}</th>
            </tr></thead>
            <tbody>
              {items.map((a) => (
                <tr key={a.alert_id} className="border-t border-edge/50 align-top">
                  <td className="py-1.5 pr-2"><SeverityBadge severity={a.severity} /></td>
                  <td className="py-1.5 pr-2 mono text-dim">{en("alertSource", a.source)}</td>
                  <td className="py-1.5 pr-2">
                    <div>{a.title}</div>
                    {a.recommended_action && <div className="mt-0.5 text-[10px] text-dim">→ {a.recommended_action}</div>}
                  </td>
                  <td className="py-1.5 pr-2 mono">{a.occurrence > 1 ? `×${a.occurrence}` : ""}</td>
                  <td className="py-1.5 pr-2"><TimeAgo ts={a.created_at} /></td>
                  <td className="py-1.5 text-right whitespace-nowrap">
                    {a.status === "open" && (
                      <>
                        <button onClick={() => act(a.alert_id, "ack")} className="mr-1 rounded border border-edge px-1.5 py-0.5 text-[10px] text-ok hover:bg-ok/10">{t("alerts.ack")}</button>
                        <button onClick={() => act(a.alert_id, "mute")} className="mr-1 rounded border border-edge px-1.5 py-0.5 text-[10px] text-dim hover:bg-edge">{t("alerts.mute")}</button>
                      </>
                    )}
                    <button
                      onClick={() => nav(`/investigations?prefill=${encodeURIComponent(`${t("alerts.prefillPrefix")}${a.title}`)}`)}
                      className="rounded border border-accent/40 px-1.5 py-0.5 text-[10px] text-accent hover:bg-accent/10"
                    >
                      {t("alerts.investigate")}
                    </button>
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
