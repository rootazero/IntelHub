// SP7 command-deck map — trimmed Radar: 24h geo events as kind-colored
// circles, click → detail card. Basemap chain (CARTO → Esri → Stadia)
// and providers are shared with Radar via ../basemap — this file only adds
// the page-specific fourth tier (bundled offline GeoJSON) past the chain.
//
// Post-2026-09-15: migrated from Leaflet to MapLibre GL JS.
//   * Each event is rendered as a MapLibre Marker wrapping a custom
//     HTML div (CSS-styled circle). HTML markers give us full control
//     over visual style + click behavior; MapLibre's GeoJSON symbol
//     layer is also viable but HTML markers are simpler at our event
//     counts (~70 typical).
//   * MapLibre's fitBounds is reliable (no stuck-render class of bugs
//     we hit with Leaflet), so the manual DOM workaround code from
//     cd8df25 is gone.
//   * All Leaflet coordinate-order gotchas (lat,lng vs lng,lat) are
//     handled at the Leaflet-callback boundary — in this file we just
//     speak [lng, lat] to MapLibre.

import { useEffect, useMemo, useRef, useState } from "react";
import { Link } from "react-router-dom";
import maplibregl, { Map as MlMap, Marker as MlMarker, type StyleSpecification } from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";
import { api } from "../../api";
import { useT } from "../../i18n";
import { kindColor, KIND_COLORS, sevRadius } from "../../kindmeta";
import { buildStyle, CHAIN, PRIMARY } from "../../basemap";
import type { TileProvider, TilesMode } from "../../basemap";
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
  const { attach } = useMapView();
  const mapRef = useRef<MlMap | null>(null);
  const markersRef = useRef<Map<string, MlMarker>>(new Map());
  const divRef = useRef<HTMLDivElement>(null);
  const bumpTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const failCount = useRef(0);
  const tileProvider = useRef<TileProvider | "offline">(PRIMARY);
  const eventsRef = useRef<GeoEvent[]>([]);
  const styleSpec = useRef<StyleSpecification | null>(null);
  const [selected, setSelected] = useState<GeoEvent | null>(null);
  const [tiles, setTiles] = useState<TilesMode>(PRIMARY);
  const [lastRefresh, setLastRefresh] = useState<number>(Date.now());
  const [bump, setBump] = useState(false);

  // ---- map init ----
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
      // We provide our own region buttons via <MapControls>; the
      // default zoom widget would compete with that, so suppress it.
    });
    mapRef.current = map;
    tileProvider.current = PRIMARY;
    styleSpec.current = buildStyle(PRIMARY);
    setTiles(PRIMARY);
    attach(map);
    // MapLibre's fitBounds runs from useMapView.flyToRegion on region
    // clicks; on initial mount we just let the default view stand.
    // The page-level region effect in useMapView handles the initial
    // fit (mount-only) — see commit cd8df25 spec.
    const ro = new ResizeObserver(() => map.resize());
    ro.observe(divRef.current);
    const tm = setTimeout(() => {
      // If 5 tiles errored, advance one tier (failover).
      if (failCount.current >= 4 && tileProvider.current !== "offline") {
        const next = CHAIN[CHAIN.indexOf(tileProvider.current as TileProvider) + 1] as TileProvider | undefined;
        if (next) switchProvider(next);
        else void goOffline();
      }
    }, 5000);
    // MapLibre fires 'idle' (not 'load') when the initial style is
    // ready; we use it for the failure-detect timer reset.
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
    return () => {
      clearTimeout(tm);
      ro.disconnect();
      attach(null);
      map.remove();
      mapRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // ---- markers ----
  function renderMarkers() {
    const map = mapRef.current;
    if (!map) return;
    // Clear existing markers.
    for (const m of markersRef.current.values()) m.remove();
    markersRef.current.clear();
    for (const ev of eventsRef.current) {
      const el = document.createElement("div");
      const size = sevRadius(ev.severity) * 2 + 4;
      el.className = ev.severity === "flash" ? "hud-marker hud-marker-pulse" : "hud-marker";
      el.style.width = `${size}px`;
      el.style.height = `${size}px`;
      el.style.background = kindColor(ev.kind);
      el.style.borderColor = kindColor(ev.kind);
      el.style.opacity = ev.severity === "info" ? "0.55" : "0.85";
      el.title = ev.title; // native browser tooltip
      const filter = selectedSource ?? null;
      const dim = filter && ev.source !== filter;
      if (dim) el.classList.add("hud-marker-dim");
      el.addEventListener("click", (e) => {
        e.stopPropagation();
        setSelected(ev);
      });
      const marker = new maplibregl.Marker({ element: el })
        .setLngLat([ev.lon, ev.lat])
        .addTo(map);
      markersRef.current.set(ev.event_id, marker);
    }
  }

  // Update dim state when SensorGrid cross-filter changes (without
  // re-rendering markers).
  useEffect(() => {
    const filter = selectedSource ?? null;
    for (const [id, marker] of markersRef.current) {
      const ev = eventsRef.current.find((e) => e.event_id === id);
      if (!ev) continue;
      const el = marker.getElement();
      if (el) el.classList.toggle("hud-marker-dim", !!(filter && ev.source !== filter));
    }
  }, [selectedSource]);

  // ---- data ----
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
      {/* MapLibre CSS sets `.maplibregl-map { position: relative }` which
          overrides our `absolute inset-0` on the same element. Wrap
          with an outer div that handles the absolute positioning. */}
      <div className="absolute inset-0">
        <div ref={divRef} style={{ width: "100%", height: "100%" }} />
      </div>

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
            <Link className="hud-btn" to={{ pathname: "/radar", search: `?event=${encodeURIComponent(selected.event_id)}` }} title={t("hud.openInRadar")}>
              ⤴ {t("hud.openInRadar")}
            </Link>
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
