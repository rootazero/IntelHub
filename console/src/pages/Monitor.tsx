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
import NewsTicker from "../components/hud/NewsTicker";
import { requireByNormalizedId } from "../lib/series_catalog";

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

// OSINT Framework bridge: per-collector state + cadence from the
// `/api/v1/health.osint_bridge` section (added 2026-09-16). Same data
// shape accept-sp6.py reads. 5 hardcoded collectors; the registry
// owner is the Rust Source impl, not this UI.
interface OsintBridgeEntry {
  name: string;
  kind: string;
  cadence_hours: number;
  state: string;
  last_fetched: number;
  last_new: number;
  ts: string;
}

// GAUGES look up series IDs from the static catalog. If a series is
// renamed or removed upstream, `requireByNormalizedId` throws at build
// time — the gauge won't silently render empty.
const GAUGES: { series: string; label: string; warnAbove?: number; warnBelow?: number }[] = [
  { series: requireByNormalizedId("fred:VIXCLS").normalized_id, label: "VIX", warnAbove: 30 },
  { series: requireByNormalizedId("fred:DGS10_PCT").normalized_id, label: "10Y %" },
  { series: requireByNormalizedId("fred:T10Y2Y").normalized_id, label: "2s10s", warnBelow: 0 },
  { series: requireByNormalizedId("fred:HY_OAS_PCT").normalized_id, label: "HY OAS" },
  { series: requireByNormalizedId("eia:WTI_SPOT_USD_BBL").normalized_id, label: "WTI $" },
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
  const [osintBridge, setOsintBridge] = useState<OsintBridgeEntry[]>([]);
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
  const loadOsintBridge = useCallback(() => {
    api<{ osint_bridge?: { collectors?: OsintBridgeEntry[] } }>("/api/v1/health")
      .then((d) => setOsintBridge(d.osint_bridge?.collectors ?? []))
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
    loadSources(); loadDelta(); loadLatest(); loadAlerts(); loadSparks(); loadOsintBridge();
    const poll = setInterval(() => { loadSources(); loadDelta(); loadLatest(); loadAlerts(); loadOsintBridge(); }, 30_000);
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

        {/* center: hero map → series strip → markets strip */}
        <div className="hud-col">
          <HudPanel title={t("hud.map")} className="flex-1" bodyClassName="!p-0 flex flex-col">
            <MonitorMap refreshKey={mapTick} selectedSource={selSource} onClearSource={() => setSelSource(null)} />
          </HudPanel>
          <HudPanel title={`${t("hud.chart")} · ${series}`} className="flex-none h-[168px]" bodyClassName="!overflow-hidden flex flex-col">
            <SeriesChart series={series} days={40} />
          </HudPanel>
          <HudPanel title={t("hud.markets")} className="flex-none h-[112px]" bodyClassName="!overflow-x-auto !overflow-y-hidden">
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

        {/* right rail: news ticker → key indicators → delta → alerts → stream */}
        <div className="hud-col">
          <HudPanel title={t("hud.news")} className="flex-none h-[150px]" bodyClassName="!overflow-hidden">
            <NewsTicker refreshKey={mapTick} />
          </HudPanel>
          {/* OSINT Framework bridge (2026-09-16): 5 hardcoded collectors that
              fill osintframework.com gaps. Reuses `/api/v1/health.osint_bridge`
              so the data and the accept-sp6.py acceptance check share one
              source of truth — no parallel fetch path. */}
          <HudPanel
            title={`OSINT Bridge (${osintBridge.filter((c) => c.state === "ok").length}/${osintBridge.length})`}
            ok={osintBridge.length > 0 && osintBridge.every((c) => c.state === "ok")}
            className="flex-none"
          >
            <div className="hud-mono text-[10px]">
              {osintBridge.map((c) => (
                <div key={c.name} className="flex items-center justify-between gap-2 py-[2px]">
                  <span className="truncate" title={c.kind} style={{ color: c.state === "ok" ? "var(--hud-ink)" : "var(--hud-warn, #f59e0b)" }}>
                    {c.name}
                  </span>
                  <span className="text-[9px]" style={{ color: "var(--hud-dim)" }}>
                    {c.cadence_hours}h
                  </span>
                  <span className="text-[9px]" style={{ color: "var(--hud-dim)" }}>
                    f={c.last_fetched} n={c.last_new}
                  </span>
                </div>
              ))}
              {osintBridge.length === 0 && <span style={{ color: "var(--hud-dim)" }}>loading…</span>}
            </div>
          </HudPanel>
          <HudPanel title={t("hud.indicators")} className="max-h-[24%]">
            <div className="hud-mono">
              {Object.values(latest)
                .filter((r) => !r.series.startsWith("quote:") && !GAUGES.some((g) => g.series === r.series))
                .sort((a, b) => a.series.localeCompare(b.series))
                .slice(0, 8)
                .map((r) => (
                  <div key={r.series} className="flex items-baseline justify-between gap-2 py-[3px] text-[11px]">
                    <span className="truncate text-[10px]" style={{ color: "var(--hud-dim)" }}>{r.series}</span>
                    <span style={{ color: "var(--hud-ink)" }}>
                      {r.value.toLocaleString(undefined, { maximumFractionDigits: 2 })}
                    </span>
                  </div>
                ))}
              {Object.keys(latest).length === 0 && (
                <span className="text-[11px]" style={{ color: "var(--hud-dim)" }}>{t("hud.noData")}</span>
              )}
            </div>
          </HudPanel>
          <HudPanel title={t("hud.delta")} className="max-h-[24%]">
            <DeltaPanel rows={delta} />
          </HudPanel>
          <HudPanel title={t("hud.alerts")} ok={alerts.length === 0} className="max-h-[20%]">
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
