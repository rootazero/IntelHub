// SP7 Monitor Command Deck — unified dark-ops HUD for the native monitor plane.
// Three rails: sensor grid + risk gauges (left), map + series chart + market
// strip (center), sweep delta + live stream + open alerts (right). Scoped
// hud.css keeps the ops aesthetic out of the rest of the console.
import { useCallback, useEffect, useRef, useState } from "react";
import { api, streamEvents, type BusEventPayload } from "../api";
import { useT } from "../i18n";
import "../hud.css";
import HudPanel from "../components/hud/Panel";
import TopStrip, { type DeltaDir } from "../components/hud/TopStrip";
import SensorGrid, { type SourceHealth } from "../components/hud/SensorGrid";
import Gauge from "../components/hud/Gauge";
import SeriesChart from "../components/hud/SeriesChart";
import DeltaPanel, { type DeltaRow } from "../components/hud/DeltaPanel";
import EventStream from "../components/hud/EventStream";
import MonitorMap from "../components/hud/MonitorMap";

interface LatestRow {
  series: string;
  observed_at: string;
  value: number;
  payload: Record<string, unknown>;
}

interface AlertRow {
  alert_id: string;
  severity: string;
  title: string;
  created_at: string;
}

const GAUGES: { series: string; label: string; warnAbove?: number; warnBelow?: number }[] = [
  { series: "fred:VIXCLS", label: "VIX", warnAbove: 30 },
  { series: "fred:DGS10", label: "10Y %" },
  { series: "fred:T10Y2Y", label: "2s10s", warnBelow: 0 },
  { series: "fred:BAMLH0A0HYM2", label: "HY OAS" },
  { series: "eia:WTI", label: "WTI $" },
];

const VISUALS_KEY = "intelhub.hud.visuals";

export default function Monitor() {
  const { t } = useT();
  const [sources, setSources] = useState<SourceHealth[]>([]);
  const [delta, setDelta] = useState<DeltaRow[]>([]);
  const [latest, setLatest] = useState<Record<string, LatestRow>>({});
  const [quotes, setQuotes] = useState<LatestRow[]>([]);
  const [sparks, setSparks] = useState<Record<string, number[]>>({});
  const [alerts, setAlerts] = useState<AlertRow[]>([]);
  const [stream, setStream] = useState<BusEventPayload[]>([]);
  const [series, setSeries] = useState(GAUGES[0].series);
  const [selSource, setSelSource] = useState<string | null>(null);
  const [mapTick, setMapTick] = useState(0);
  const [visuals, setVisuals] = useState<"full" | "lite">(
    () => (localStorage.getItem(VISUALS_KEY) as "full" | "lite") || (window.innerWidth < 1100 ? "lite" : "full"),
  );
  const seen = useRef(new Set<string>());

  const loadSources = useCallback(() => {
    api<{ radar?: { monitor?: { sources?: SourceHealth[] } } }>("/api/v1/overview")
      .then((d) => setSources(d.radar?.monitor?.sources ?? []))
      .catch(() => {});
  }, []);
  const loadDelta = useCallback(() => {
    api<{ sources: DeltaRow[] }>("/api/v1/monitor/delta")
      .then((d) => setDelta(d.sources ?? []))
      .catch(() => {});
  }, []);
  const loadLatest = useCallback(() => {
    api<{ series: LatestRow[] }>("/api/v1/signals/latest")
      .then((d) => {
        const map: Record<string, LatestRow> = {};
        const qs: LatestRow[] = [];
        for (const row of d.series ?? []) {
          map[row.series] = row;
          if (row.series.startsWith("quote:")) qs.push(row);
        }
        setLatest(map);
        setQuotes(qs.slice(0, 12));
      })
      .catch(() => {});
  }, []);
  const loadAlerts = useCallback(() => {
    api<{ items?: AlertRow[]; alerts?: AlertRow[] }>("/api/v1/alerts?status=open")
      .then((d) => setAlerts((d.items ?? d.alerts ?? []).slice(0, 5)))
      .catch(() => {});
  }, []);
  const loadSparks = useCallback(() => {
    GAUGES.forEach((g) => {
      api<{ points: { t: string; v: number }[] }>(
        `/api/v1/signals/history?series=${encodeURIComponent(g.series)}&days=40`,
      )
        .then((d) => setSparks((prev) => ({ ...prev, [g.series]: (d.points ?? []).map((p) => p.v) })))
        .catch(() => {});
    });
  }, []);

  useEffect(() => {
    loadSources(); loadDelta(); loadLatest(); loadAlerts(); loadSparks();
    const poll = setInterval(() => { loadSources(); loadDelta(); loadLatest(); loadAlerts(); }, 30_000);
    const stop = streamEvents((ev) => {
      if (seen.current.has(ev.event_id)) return;
      seen.current.add(ev.event_id);
      if (ev.event_type === "monitor_sweep_ingested") {
        loadDelta(); loadSources();
        const p = ev.payload as { new?: number };
        if ((p.new ?? 0) > 0) {
          setStream((prev) => [ev, ...prev].slice(0, 30));
          setMapTick((k) => k + 1);
        }
      } else if (ev.event_type === "alert_raised") {
        loadAlerts();
        setStream((prev) => [ev, ...prev].slice(0, 30));
      }
    });
    return () => { clearInterval(poll); stop(); };
  }, [loadSources, loadDelta, loadLatest, loadAlerts, loadSparks]);

  const toggleVisuals = () => {
    const next = visuals === "full" ? "lite" : "full";
    setVisuals(next);
    localStorage.setItem(VISUALS_KEY, next);
  };

  const ok = sources.filter((s) => s.state === "ok").length;
  const dir: DeltaDir = (() => {
    if (delta.length === 0) return "flat";
    if (delta.every((r) => r.direction === "new_source")) return "new_source";
    const up = delta.some((r) => r.direction === "up");
    const down = delta.some((r) => r.direction === "down");
    return up && down ? "mixed" : up ? "up" : down ? "down" : "flat";
  })();
  const lastSweep = delta.map((r) => r.ts).filter(Boolean).sort().pop() ?? null;

  const changePct = (row: LatestRow): number | null => {
    const p = row.payload ?? {};
    const v = (p.change_pct ?? p.changePct) as number | undefined;
    return typeof v === "number" ? v : null;
  };

  return (
    <div className={`hud-root ${visuals === "lite" ? "hud-lite" : ""}`}>
      <TopStrip
        sourcesOk={ok}
        sourcesTotal={sources.length}
        direction={dir}
        lastSweep={lastSweep}
        openAlerts={alerts.length}
        visuals={visuals}
        onToggleVisuals={toggleVisuals}
      />
      <div className="hud-grid">
        {/* left rail */}
        <div className="hud-col">
          <HudPanel title={t("hud.sensorGrid")} ok={ok === sources.length && sources.length > 0} className="max-h-[46%]">
            <SensorGrid sources={sources} selected={selSource} onSelect={setSelSource} />
          </HudPanel>
          <HudPanel title={t("hud.gauges")} className="flex-1">
            {GAUGES.map((g) => {
              const v = latest[g.series]?.value ?? null;
              const warn =
                v !== null &&
                ((g.warnAbove !== undefined && v > g.warnAbove) || (g.warnBelow !== undefined && v < g.warnBelow));
              return (
                <Gauge
                  key={g.series}
                  label={g.label}
                  value={v}
                  points={sparks[g.series] ?? []}
                  warn={warn}
                  active={series === g.series}
                  onClick={() => setSeries(g.series)}
                />
              );
            })}
          </HudPanel>
        </div>

        {/* center */}
        <div className="hud-col">
          <HudPanel title={t("hud.map")} className="flex-[3]" bodyClassName="!p-0 flex flex-col">
            <MonitorMap refreshKey={mapTick} />
          </HudPanel>
          <HudPanel title={`${t("hud.chart")} · ${series}`} className="flex-[2]" bodyClassName="!overflow-hidden flex flex-col">
            <SeriesChart series={series} days={40} />
          </HudPanel>
          <HudPanel title={t("hud.markets")} bodyClassName="!overflow-x-auto !overflow-y-hidden">
            <div className="flex gap-2">
              {quotes.map((q) => {
                const cp = changePct(q);
                return (
                  <div key={q.series} className="hud-market-card">
                    <div className="text-[9px] uppercase tracking-wider" style={{ color: "var(--hud-dim)" }}>
                      {q.series.replace("quote:", "")}
                    </div>
                    <div className="hud-mono text-[14px]">{q.value.toLocaleString(undefined, { maximumFractionDigits: 2 })}</div>
                    {cp !== null && (
                      <div className={`hud-mono text-[10px] ${cp >= 0 ? "hud-down" : "hud-up"}`}>
                        {cp >= 0 ? "+" : ""}{cp.toFixed(2)}%
                      </div>
                    )}
                  </div>
                );
              })}
              {quotes.length === 0 && <span className="text-[11px]" style={{ color: "var(--hud-dim)" }}>{t("hud.noData")}</span>}
            </div>
          </HudPanel>
        </div>

        {/* right rail */}
        <div className="hud-col">
          <HudPanel title={t("hud.delta")} className="max-h-[34%]">
            <DeltaPanel rows={delta} />
          </HudPanel>
          <HudPanel title={t("hud.alerts")} ok={alerts.length === 0} className="max-h-[30%]">
            {alerts.map((a) => (
              <div key={a.alert_id} className="hud-stream-card alert">
                <div className="hud-mono text-[9px] uppercase" style={{ color: "var(--hud-dim)" }}>{a.severity}</div>
                <div className="truncate" title={a.title}>{a.title}</div>
              </div>
            ))}
            {alerts.length === 0 && <div className="py-1 text-[11px]" style={{ color: "var(--hud-dim)" }}>{t("hud.noData")}</div>}
          </HudPanel>
          <HudPanel title={t("hud.stream")} className="flex-1">
            <EventStream events={stream} />
          </HudPanel>
        </div>
      </div>
    </div>
  );
}
