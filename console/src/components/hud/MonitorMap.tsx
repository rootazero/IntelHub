// SP7 command-deck map — trimmed Radar: 24h geo events as kind-colored
// circleMarkers, click → detail card. Basemap chain (CARTO → Esri → Stadia)
// and providers are shared with Radar via ../basemap — this file only adds
// the page-specific fourth tier (bundled offline GeoJSON) past the chain.
//
// Monitor-vs-Radar deltas:
//   * Region POV (6 presets: world + 5 continents) — quick focus from the deck
//   * SensorGrid cross-filter — when a source is selected, others dim, match pops
//   * Pulse animation for flash severity (Crucix conflict-rings feel, 2D)
//   * Map controls (zoom +/-, region reset) — top-right
//   * Tile-mode + event-count + last-refresh status pills — top-left
//   * Compact kind legend — bottom-right, bumps on fresh SSE sweep
//   * Detail card with payload peek + "open in Radar" deep link
//
// No filter UI (Radar page is the deep view); all interactive elements here are
// "what's where" not "what to look at".
import { useEffect, useMemo, useRef, useState } from "react";
import L from "leaflet";
import "leaflet/dist/leaflet.css";
import { api } from "../../api";
import { useT } from "../../i18n";
import { kindColor, KIND_COLORS, sevRadius } from "../../kindmeta";
import { PROVIDERS, CHAIN, PRIMARY } from "../../basemap";
import type { TileProvider, TilesMode } from "../../basemap";
import { REGIONS } from "../../mapControls";
import { useMapView } from "../../useMapView";
import { MapControls } from "../MapControls";

interface GeoEvent {
  event_id: string;
  source: string;
  kind: string;
  title: string;
  lat: number;
  lon: number;
  severity: string;
  occurred_at: string;
  payload?: Record<string, unknown>;
}

const SEV_COLOR: Record<string, string> = {
  flash: "#ff3355",
  priority: "#ff9500",
  routine: "#ffd60a",
  info: "#38bdf8",
};

// Frame↔map contract: the map always shows the INHABITED world fitted exactly
// into the card — longitude spans the full 359° so no repeated continent
// copies can ever render; only uninhabited polar/pacific fringes are trimmed;
// every continent (incl. East Asia) stays whole. Fractional zoomSnap 0.25 lets
// fitBounds fill any panel aspect precisely. Leaflet does not track container
// resizes by itself — invalidateSize() before every refit.
const WORLD: L.LatLngBoundsExpression = [[-58, -179], [76, 180]];

// Region POVs live in ../../mapControls (single source of truth shared with
// Radar); 6 preset angles with latitudes slightly trimmed so polar cap labels
// don't crowd and longitudes centered on each region.

export default function MonitorMap({
  refreshKey,
  selectedSource,
  onClearSource,
}: {
  refreshKey: number;
  selectedSource?: string | null;
  onClearSource?: () => void;
}) {
  const { t } = useT();
  const { region, attach } = useMapView();
  const mapRef = useRef<L.Map | null>(null);
  const layerRef = useRef<L.LayerGroup | null>(null);
  const baseRef = useRef<L.Layer | null>(null);
  const divRef = useRef<HTMLDivElement>(null);
  const markersRef = useRef<Map<string, L.CircleMarker>>(new Map());
  const bumpTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const failCount = useRef(0);
  const tileProvider = useRef<TileProvider>(PRIMARY);
  const offlineGeo = useRef<GeoJSON.GeoJSON | null>(null);
  const eventsRef = useRef<GeoEvent[]>([]);
  // skip the first run: the map-init effect below already fitBounds to
  // REGIONS[region] (initial = 'world'); calling flyToBounds again here
  // would hit Leaflet before the init RAF has set the view → 'Set map
  // center and zoom first' error, React unmounts the whole tree.
  const skipFirstRegion = useRef(true);
  const [selected, setSelected] = useState<GeoEvent | null>(null);
  const [tiles, setTiles] = useState<TilesMode>(PRIMARY);
  const [lastRefresh, setLastRefresh] = useState<number>(Date.now());
  const [bump, setBump] = useState(false);

  function addTileLayer(map: L.Map, provider: TileProvider) {
    const p = PROVIDERS[provider];
    const layer = L.tileLayer(p.url, p.options);
    layer.on("tileerror", () => { failCount.current += 1; if (failCount.current >= 4) failover(); });
    layer.on("tileload", () => { failCount.current = 0; });
    baseRef.current = layer;
    layer.addTo(map);
    tileProvider.current = provider;
    setTiles(provider);
  }

  function failover() {
    const map = mapRef.current;
    if (!map) return;
    const next = CHAIN[CHAIN.indexOf(tileProvider.current) + 1] as TileProvider | undefined;
    if (next) {
      if (baseRef.current) { map.removeLayer(baseRef.current); baseRef.current = null; }
      failCount.current = 0;
      addTileLayer(map, next);
    } else {
      void goOffline();
    }
  }

  async function goOffline() {
    const map = mapRef.current;
    if (!map) return;
    if (baseRef.current) { map.removeLayer(baseRef.current); baseRef.current = null; }
    if (!offlineGeo.current) {
      try {
        const r = await fetch("/world-110m.geo.json");
        offlineGeo.current = (await r.json()) as GeoJSON.GeoJSON;
      } catch { return; }
    }
    baseRef.current = L.geoJSON(offlineGeo.current, {
      style: { color: "#2a3b4d", weight: 0.8, fillColor: "#101820", fillOpacity: 0.85 },
    });
    baseRef.current.addTo(map);
    setTiles("offline");
  }

  useEffect(() => {
    if (!divRef.current || mapRef.current) return;
    const map = L.map(divRef.current, {
      zoomSnap: 0.25, zoomDelta: 0.25,
      minZoom: 1, maxZoom: 10,
      worldCopyJump: true,
      attributionControl: false,
      zoomControl: false,
      maxBounds: [[-85, -180], [85, 180]],
      maxBoundsViscosity: 1.0,
    });
    mapRef.current = map;
    layerRef.current = L.layerGroup().addTo(map);
    attach(map);
    addTileLayer(map, PRIMARY);
    const fit = () => {
      map.invalidateSize();
      if ((divRef.current?.clientWidth ?? 0) > 0) map.fitBounds(REGIONS[region], { animate: false });
    };
    const raf = requestAnimationFrame(fit);
    const settle = setTimeout(fit, 300);
    const ro = new ResizeObserver(fit);
    ro.observe(divRef.current);
    const tm = setTimeout(() => { if (failCount.current > 0 && baseRef.current) failover(); }, 5000);
    return () => {
      clearTimeout(tm);
      clearTimeout(settle);
      cancelAnimationFrame(raf);
      ro.disconnect();
      attach(null);
    };
    // region is intentionally NOT in deps — initial mount only; region
    // changes are handled by a dedicated effect (smoother, separate animate).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Smooth region transition when user clicks a region button. Re-fits the
  // current region (no flicker because we keep the layer group intact).
  useEffect(() => {
    if (skipFirstRegion.current) { skipFirstRegion.current = false; return; }
    const map = mapRef.current;
    if (!map) return;
    map.invalidateSize();
    map.flyToBounds(REGIONS[region], { duration: 0.6, easeLinearity: 0.3 });
  }, [region]);

  function renderMarkers() {
    const lg = layerRef.current;
    if (!lg) return;
    lg.clearLayers();
    markersRef.current.clear();
    for (const ev of eventsRef.current) {
      const isFlash = ev.severity === "flash";
      const m = L.circleMarker([ev.lat, ev.lon], {
        radius: sevRadius(ev.severity) - 1,
        color: kindColor(ev.kind),
        weight: isFlash ? 2 : 1,
        fillColor: kindColor(ev.kind),
        fillOpacity: ev.severity === "info" ? 0.35 : 0.7,
        className: isFlash ? "hud-marker-pulse" : "",
      });
      m.on("click", () => setSelected(ev));
      m.bindTooltip(ev.title, { direction: "top", offset: [0, -4] });
      lg.addLayer(m);
      markersRef.current.set(ev.event_id, m);
    }
    applySourceFilter();
  }

  // When SensorGrid selects a source, dim non-matching markers visually
  // without removing them (so user sees context + filtered foreground).
  function applySourceFilter() {
    const filter = selectedSource ?? null;
    for (const [id, m] of markersRef.current) {
      const ev = eventsRef.current.find((e) => e.event_id === id);
      if (!ev) continue;
      const dim = filter && ev.source !== filter;
      const el = m.getElement();
      if (el) el.classList.toggle("hud-marker-dim", !!dim);
    }
  }

  useEffect(() => { applySourceFilter(); }, [selectedSource]);

  useEffect(() => {
    let live = true;
    const from = new Date(Date.now() - 24 * 3600_000).toISOString();
    api<{ items: GeoEvent[] }>(`/api/v1/radar/events?from=${encodeURIComponent(from)}&limit=800`)
      .then((d) => {
        if (!live) return;
        eventsRef.current = d.items ?? [];
        renderMarkers();
        setLastRefresh(Date.now());
        // refreshKey only changes when the SSE stream reports new events, so
        // every successful fetch here IS a fresh sweep — bump unconditionally
        // to give the deck a visible confirmation.
        setBump(true);
        if (bumpTimer.current) clearTimeout(bumpTimer.current);
        bumpTimer.current = setTimeout(() => setBump(false), 900);
      })
      .catch(() => {});
    return () => {
      live = false;
      if (bumpTimer.current) clearTimeout(bumpTimer.current);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refreshKey]);

  // Top-5 kinds by current event set → legend. Sorted by count desc so the
  // most active signals surface first (Crucix-style).
  const legend = useMemo(() => {
    const counts = new Map<string, number>();
    for (const ev of eventsRef.current) counts.set(ev.kind, (counts.get(ev.kind) ?? 0) + 1);
    return Array.from(counts.entries())
      .sort((a, b) => b[1] - a[1])
      .slice(0, 5)
      .map(([kind, n]) => ({ kind, n }));
  }, [lastRefresh]); // eslint-disable-line react-hooks/exhaustive-deps

  const tileLabel = tiles === PRIMARY ? t("hud.tilesOnline") : tiles === "esri" ? t("hud.tilesEsri") : t("hud.tilesOffline");
  const tileClass = tiles === PRIMARY ? "online" : tiles === "esri" ? "fallback" : "offline";

  return (
    <div className="hud-map-wrap">
      <div ref={divRef} className="absolute inset-0" />

      {/* status pills: top-left. Bumps when fresh SSE sweep lands */}
      <div className="hud-map-status">
        <span className={`hud-map-pill ${tileClass} ${bump ? "bump" : ""}`}>
          {tileLabel}
        </span>
        <span className={`hud-map-pill ${bump ? "bump" : ""}`}>
          {t("hud.lastSweep")} <span className="v"><TimeAgoMini ts={lastRefresh} /></span>
        </span>
        <span className={`hud-map-pill ${bump ? "bump" : ""}`}>
          {eventsRef.current.length} <span className="v">{eventsRef.current.length === 1 ? "evt" : "evts"}</span>
        </span>
      </div>

      {/* map controls: top-right. Region jump (single row) + zoom + reset.
          Shared with Radar via <MapControls> — single behavior, two themes. */}
      <MapControls className="hud-map-ctrl" />

      {/* layer legend: bottom-right. Bumps on fresh sweep. */}
      <div className={`hud-map-legend ${bump ? "hud-legend-bump" : ""}`}>
        {legend.length === 0 ? (
          <span style={{ color: "var(--hud-dim)" }}>{t("hud.noEvents")}</span>
        ) : legend.map((row) => (
          <div key={row.kind} className="hud-map-legend-row">
            <span className="hud-map-legend-dot" style={{ background: KIND_COLORS[row.kind] ?? "#8a8f98", color: KIND_COLORS[row.kind] ?? "#8a8f98" }} />
            <span>{row.kind}</span>
            <span style={{ color: "var(--hud-ink)", fontWeight: 600 }}>{row.n}</span>
          </div>
        ))}
      </div>

      {/* source filter badge: bottom-left when SensorGrid cross-filter is active */}
      {selectedSource && (
        <div className="hud-map-filter">
          <span>{t("hud.filteredBy", { source: selectedSource })}</span>
          <button className="hud-map-filter-x" title={t("hud.clearFilter")} onClick={() => onClearSource?.()}>×</button>
        </div>
      )}

      {/* detail card: payload peek + open-in-Radar deep link */}
      {selected && (
        <div className="hud-map-detail">
          <div className="hud-map-detail-head">
            <span className="hud-dot" style={{ background: SEV_COLOR[selected.severity] ?? "#38bdf8" }} />
            <span className="hud-mono text-[9px] uppercase" style={{ color: "var(--hud-dim)" }}>
              {selected.kind} · {selected.source}
            </span>
            <span className="flex-1" />
            <button className="hud-btn" onClick={() => setSelected(null)} title="Close">×</button>
          </div>
          <div className="hud-map-detail-title">{selected.title}</div>
          <div className="hud-map-detail-meta">
            <span>SEV {selected.severity}</span>
            <span>{selected.lat.toFixed(2)}, {selected.lon.toFixed(2)}</span>
            <span><TimeAgoMini ts={Date.parse(selected.occurred_at)} /></span>
          </div>
          {selected.payload && Object.keys(selected.payload).length > 0 && (
            <pre className="hud-map-detail-payload">
              {JSON.stringify(selected.payload, null, 2).slice(0, 600)}
            </pre>
          )}
          <div className="hud-map-detail-actions">
            <a className="hud-btn" href="#/radar" title={t("hud.openInRadar")}>
              ⤴ {t("hud.openInRadar")}
            </a>
          </div>
        </div>
      )}
    </div>
  );
}

// Local "time-ago" for the status pill — different cadence (last refresh
// is seconds, not "minutes ago") and smaller than the shared TimeAgo.
function TimeAgoMini({ ts }: { ts: number }) {
  const [, force] = useState(0);
  useEffect(() => {
    const id = setInterval(() => force((n) => n + 1), 15_000);
    return () => clearInterval(id);
  }, []);
  const s = Math.max(0, (Date.now() - ts) / 1000);
  const label = s < 60 ? `${Math.floor(s)}s` : `${Math.floor(s / 60)}m`;
  return <span>{label}</span>;
}