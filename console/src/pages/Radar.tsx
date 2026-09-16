// Radar (§26) — global signal map fed by hub geo_events (native monitor sweeps).
// Dual basemap: CARTO dark tiles by default; on tile failure burst or first
// load timeout, falls back to the bundled offline world vector layer.
//
// Post-2026-09-15: migrated from Leaflet to MapLibre GL JS.
//   * HTML markers (custom div + new maplibregl.Marker) instead of
//     L.circleMarker. Each marker is a separate DOM element, allowing
//     full CSS control (pulse animation, dim filter, etc.).
//   * fitBounds for region navigation is reliable on MapLibre — the
//     old Leaflet "stuck-render" bug (commits cd8df25 and earlier)
//     is gone. The deepLinkFocused ref-flag race workaround is also
//     gone since the underlying bug no longer exists.
//   * All coordinate-order gotchas (lat,lng vs lng,lat) are handled at
//     the Leaflet-callback boundary — in this file we just speak
//     [lng, lat] to MapLibre.

import { useEffect, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";
import maplibregl, { Map as MlMap, Marker as MlMarker, type StyleSpecification } from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";
import { api, streamEvents } from "../api";
import { useEnum, useT } from "../i18n";
import { KINDS, kindColor, sevRadius } from "../kindmeta";
import { focusOnEvent } from "../lib/eventFocus";
import { buildStyle, CHAIN, PRIMARY, STADIA_KEY, CARTO_KEY } from "../basemap";
import type { TileProvider, TilesMode } from "../basemap";
import { useMapView } from "../useMapView";
import { MapControls } from "../components/MapControls";

interface GeoEvent {
  event_id: string;
  source: string;
  kind: string;
  title: string;
  lat: number;
  lon: number;
  severity: string;
  occurred_at: string;
  payload: Record<string, unknown>;
}

const SEV_COLOR: Record<string, string> = {
  flash: "#ff3355",
  priority: "#ff9500",
  routine: "#ffd60a",
  info: "#38bdf8",
};

const WINDOWS: Record<string, number> = { "1h": 1, "24h": 24, "72h": 72, "7d": 168, "30d": 720 };

// A burst of ≥4 tile errors (or no successful tile in the first 5s)
// advances one tier; the last tier is the bundled offline GeoJSON.
// Basemap providers and chain order are defined in ../basemap (single
// source of truth shared with the command-deck MonitorMap).

export default function Radar() {
  const { t } = useT();
  const en = useEnum();
  const { region, attach, registerOnReset } = useMapView();
  const mapRef = useRef<MlMap | null>(null);
  const markersRef = useRef<Map<string, MlMarker>>(new Map());
  const divRef = useRef<HTMLDivElement>(null);
  const [events, setEvents] = useState<GeoEvent[]>([]);
  const [selected, setSelected] = useState<GeoEvent | null>(null);
  const [tiles, setTiles] = useState<TilesMode>(PRIMARY);
  const [window_, setWindow_] = useState("24h");
  const [sev, setSev] = useState("");
  const [kind, setKind] = useState("");
  const [creating, setCreating] = useState(false);
  // Deep-link: /radar?event=<id> arrives from the Monitor Command Deck's
  // "open in Radar" button. After focus, we strip the query so the user
  // owns the URL and a manual refresh doesn't re-center.
  const [searchParams, setSearchParams] = useSearchParams();
  const deepLinkEventId = searchParams.get("event");
  const [deepLinkMissing, setDeepLinkMissing] = useState<string | null>(null);
  const failCount = useRef(0);
  const tileProvider = useRef<TileProvider | "offline">(PRIMARY);
  const styleSpec = useRef<StyleSpecification | null>(null);

  // ---- map init (once) ----
  useEffect(() => {
    if (!divRef.current || mapRef.current) return;
    const map = new maplibregl.Map({
      container: divRef.current,
      style: buildStyle(PRIMARY),
      center: [10, 25],
      zoom: 1.5,
      minZoom: 1,
      maxZoom: 10,
      attributionControl: { compact: true },
      // We provide our own zoom widget via <MapControls>; the default
      // zoom widget would compete with that, so suppress it.
    });
    mapRef.current = map;
    styleSpec.current = buildStyle(PRIMARY);
    setTiles(PRIMARY);
    tileProvider.current = PRIMARY;
    attach(map);
    // When the ⌂ reset button is clicked (or any other consumer of
    // useMapView.reset()), also close the right drawer — the user is
    // explicitly leaving the focused event behind.
    const unregisterReset = registerOnReset(() => setSelected(null));
    // Resize observer: MapLibre doesn't auto-resize when the container
    // size changes. Call map.resize() whenever the panel reflows.
    const ro = new ResizeObserver(() => map.resize());
    ro.observe(divRef.current);
    // First-load timeout: if tiles are erroring after 5s, advance one
    // tier. The MapLibre 'error' event covers tile fetch failures.
    const tm = setTimeout(() => {
      if (failCount.current > 0 && tileProvider.current !== "offline") {
        const next = CHAIN[CHAIN.indexOf(tileProvider.current as TileProvider) + 1] as TileProvider | undefined;
        if (next) switchProvider(next);
        else void goOffline();
      }
    }, 5000);
    function switchProvider(next: TileProvider) {
      const m = mapRef.current;
      if (!m) return;
      tileProvider.current = next;
      styleSpec.current = buildStyle(next);
      m.setStyle(buildStyle(next), { diff: false });
      setTiles(next);
      failCount.current = 0;
    }
    async function goOffline() {
      const m = mapRef.current;
      if (!m) return;
      tileProvider.current = "offline";
      m.setStyle(buildStyle("offline"), { diff: false });
      setTiles("offline");
    }
    map.on("idle", () => { failCount.current = 0; });
    map.on("error", (e) => {
      // Tile fetch errors increment a counter; once it crosses the
      // threshold, we advance the chain.
      failCount.current += 1;
      if (failCount.current >= 4) {
        const next = CHAIN[CHAIN.indexOf(tileProvider.current as TileProvider) + 1] as TileProvider | undefined;
        if (next) switchProvider(next);
        else void goOffline();
      }
    });
    return () => {
      clearTimeout(tm);
      ro.disconnect();
      unregisterReset();
      attach(null);
      map.remove();
      mapRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // ---- data ----
  // Kind filtering is CLIENT-side: the bottom chips double as a live legend,
  // so counts must be computed from the full (kind-unfiltered) event set.
  async function load() {
    const hours = WINDOWS[window_];
    const from = new Date(Date.now() - hours * 3600_000).toISOString();
    const params = new URLSearchParams({ from, limit: "1500" });
    if (sev) params.set("severity", sev);
    try {
      const d = await api<{ items: GeoEvent[] }>(`/api/v1/radar/events?${params}`);
      setEvents(d.items);
    } catch {
      /* keep stale */
    }
  }

  const visible = kind ? events.filter((e) => e.kind === kind) : events;

  useEffect(() => {
    load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [window_, sev]);

  // Deep-link focus: when /radar?event=<id> arrives, find the event in
  // the current `events` list, open the right drawer, and flyTo it. Strip
  // the query immediately so a refresh or a subsequent click doesn't
  // re-trigger the flyTo (and so the URL is clean for sharing).
  useEffect(() => {
    if (!deepLinkEventId) return;
    if (events.length === 0) return; // wait for first load
    const ev = events.find((e) => e.event_id === deepLinkEventId);
    setSearchParams({}, { replace: true }); // strip ?event= regardless
    if (!ev) {
      setDeepLinkMissing(deepLinkEventId);
      return;
    }
    setDeepLinkMissing(null);
    // Shared utility — same behavior as the click handler, so the
    // deep-link landing and a direct page-2 click feel identical.
    setSelected(ev);
    focusOnEvent(mapRef.current, ev);
  }, [deepLinkEventId, events.length, setSearchParams]);

  // SSE: refetch on new sweep ingestion
  useEffect(() => {
    const stop = streamEvents((ev) => {
      if (ev.event_type === "monitor_sweep_ingested") load();
    });
    return stop;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [window_, sev]);

  // ---- markers ----
  useEffect(() => {
    const map = mapRef.current;
    if (!map) return;
    // Clear existing markers.
    for (const m of markersRef.current.values()) m.remove();
    markersRef.current.clear();
    for (const e of visible) {
      const el = document.createElement("div");
      const size = sevRadius(e.severity) * 2 + 6;
      el.className = e.severity === "flash" ? "radar-marker radar-marker-flash" : "radar-marker";
      el.style.width = `${size}px`;
      el.style.height = `${size}px`;
      el.style.background = kindColor(e.kind);
      el.style.borderColor = kindColor(e.kind);
      el.style.opacity = e.severity === "info" ? "0.55" : "0.85";
      el.title = `${e.kind} · ${e.title}`;
      el.addEventListener("click", (ev) => {
        ev.stopPropagation();
        setSelected(e);
        focusOnEvent(mapRef.current, e);
      });
      const marker = new maplibregl.Marker({ element: el })
        .setLngLat([e.lon, e.lat])
        .addTo(map);
      markersRef.current.set(e.event_id, marker);
    }
  }, [visible]);

  async function toInvestigation(e: GeoEvent) {
    setCreating(true);
    try {
      const inv = await api<{ investigation_id: string }>("/api/v1/investigations", {
        method: "POST",
        body: JSON.stringify({
          title: `[Radar] ${e.title}`.slice(0, 180),
          target: `${e.kind} @ ${e.lat.toFixed(2)},${e.lon.toFixed(2)}`,
          hypothesis: `Radar signal ${e.event_id} (source ${e.source}, severity ${e.severity}) requires investigation.`,
          source_event_id: e.event_id,
        }),
      });
      window.location.href = `/investigations/${inv.investigation_id}`;
    } catch {
      setCreating(false);
    }
  }

  const counts = events.reduce<Record<string, number>>((acc, e) => {
    acc[e.kind] = (acc[e.kind] ?? 0) + 1;
    return acc;
  }, {});

  return (
    <div className="flex h-full flex-col">
      <div className="flex flex-wrap items-center gap-2 border-b border-edge bg-panel px-3 py-2 text-xs">
        <span className="font-semibold">{t("radar.title")}</span>
        <span className="text-dim">{t("radar.subtitle")}</span>
        <select className="rounded border border-edge bg-base px-1.5 py-0.5" value={window_} onChange={(e) => setWindow_(e.target.value)}>
          {Object.keys(WINDOWS).map((w) => (
            <option key={w}>{w}</option>
          ))}
        </select>
        <select className="rounded border border-edge bg-base px-1.5 py-0.5" value={sev} onChange={(e) => setSev(e.target.value)}>
          <option value="">{t("radar.allSeverity")}</option>
          {["flash", "priority", "routine", "info"].map((s) => (
            <option key={s} value={s}>{en("severity", s)}</option>
          ))}
        </select>
        <select className="rounded border border-edge bg-base px-1.5 py-0.5" value={kind} onChange={(e) => setKind(e.target.value)}>
          <option value="">{t("radar.allKinds")}</option>
          {KINDS.map((k) => (
            <option key={k} value={k}>{en("kind", k)}</option>
          ))}
        </select>
        <span className="text-dim">{t("radar.events", { n: visible.length })}</span>
        <span className="ml-auto flex items-center gap-2">
          {deepLinkMissing && (
            <span
              className="rounded bg-amber-500/15 px-1.5 py-0.5 mono text-[10px] text-amber-300"
              title="Try a wider time window (7d) from the dropdown above if the event is older than the default 24h."
            >
              {t("radar.deepLinkMissing", { id: deepLinkMissing.slice(0, 8) })}
              <button
                onClick={() => setDeepLinkMissing(null)}
                className="ml-1 text-amber-200 hover:text-amber-100"
                title="Dismiss"
              >
                ✕
              </button>
            </span>
          )}
          {!STADIA_KEY && !CARTO_KEY && (
            <span
              className="rounded bg-rose-500/15 px-1.5 py-0.5 mono text-[10px] text-rose-400"
              title="Dark basemap key missing from build — chain starts at Esri fallback (grey not black). Sign up free at carto.com/basemaps/apikey (5M tiles/mo, preferred) or stadiamaps.com (200K credits/mo), then rebuild the console with VITE_CARTO_KEY or VITE_STADIA_KEY set."
            >
              DARK MAP KEY MISSING
            </span>
          )}
          <span
            className={`rounded px-1.5 py-0.5 mono text-[10px] ${tiles === PRIMARY ? "bg-emerald-500/15 text-emerald-400" : "bg-amber-500/15 text-amber-400"}`}
          >
            {tiles === PRIMARY ? t("radar.tilesOnline") : tiles === "esri" ? t("radar.tilesEsri") : t("radar.tilesOffline")}
          </span>
          {tiles !== PRIMARY && (
            <button className="rounded border border-edge px-1.5 py-0.5 text-[10px] hover:border-accent" onClick={() => {
              const map = mapRef.current;
              if (!map) return;
              tileProvider.current = PRIMARY;
              map.setStyle(buildStyle(PRIMARY), { diff: false });
              setTiles(PRIMARY);
              failCount.current = 0;
            }}>
              {t("radar.retryOnline")}
            </button>
          )}
        </span>
      </div>
      <div className="flex min-h-0 flex-1">
        <div className="relative min-w-0 flex-1">
          <div ref={divRef} className="absolute inset-0" style={{ background: "#0a0e14" }} />
          {/* map controls: top-right. Shared with MonitorMap via <MapControls>;
              one source of truth (region presets + zoom + reset). */}
          <MapControls className="radar-map-ctrl" />
        </div>
        {selected && (
          <aside className="w-80 shrink-0 overflow-y-auto border-l border-edge bg-panel p-3 text-xs">
            <div className="mb-2 flex items-center justify-between">
              <span className="font-semibold" style={{ color: SEV_COLOR[selected.severity] ?? SEV_COLOR.info }}>
                {en("severity", selected.severity).toUpperCase()} · {en("kind", selected.kind)}
              </span>
              <button className="text-dim hover:text-ink" onClick={() => setSelected(null)} title="Close">✕</button>
            </div>
            <div className="mb-1 font-medium">{selected.title}</div>
            <div className="mb-2 text-dim">
              {selected.source} · {new Date(selected.occurred_at).toLocaleString()}
              <br />
              {selected.lat.toFixed(3)}, {selected.lon.toFixed(3)}
            </div>
            <pre className="mb-3 max-h-64 overflow-auto rounded border border-edge bg-base p-2 text-[10px] mono">
              {JSON.stringify(selected.payload, null, 2).slice(0, 2000)}
            </pre>
            <button
              disabled={creating}
              onClick={() => toInvestigation(selected)}
              className="w-full rounded bg-accent/20 px-2 py-1.5 font-semibold text-accent hover:bg-accent/30 disabled:opacity-40"
            >
              {creating ? t("radar.creating") : t("radar.toInvestigation")}
            </button>
          </aside>
        )}
      </div>
      <div className="flex flex-wrap gap-1.5 border-t border-edge bg-panel px-3 py-1.5 text-[10px]">
        <button
          onClick={() => setKind("")}
          className={`flex items-center gap-1 rounded border px-1.5 py-0.5 transition-colors ${
            kind === "" ? "border-edge bg-white/10" : "border-transparent hover:bg-white/5"
          }`}
        >
          <span className={kind === "" ? "text-ink" : "text-dim"}>{t("radar.allKinds")}</span>
          <span className="text-ink font-medium">{events.length}</span>
        </button>
        {Object.entries(counts).sort((a, b) => b[1] - a[1]).map(([k, n]) => (
          <button
            key={k}
            onClick={() => setKind(kind === k ? "" : k)}
            className={`flex items-center gap-1 rounded border px-1.5 py-0.5 transition-colors ${
              kind === k ? "border-edge bg-white/10" : "border-transparent hover:bg-white/5"
            }`}
            title={kind === k ? t("radar.allKinds") : en("kind", k)}
          >
            <span className="inline-block h-2 w-2 rounded-full" style={{ background: kindColor(k) }} />
            <span className={kind === k ? "text-ink" : "text-dim"}>{en("kind", k)}</span>
            <span className="text-ink font-medium">{n}</span>
          </button>
        ))}
      </div>
    </div>
  );
}
