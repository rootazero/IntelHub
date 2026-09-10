// SP7 command-deck map — trimmed Radar: same 3-tier basemap failover chain
// (CARTO key → Esri → bundled offline GeoJSON), 24h geo events as kind-colored
// circleMarkers, click → detail card. No filter UI (Radar page is the deep view).
import { useEffect, useRef, useState } from "react";
import L from "leaflet";
import "leaflet/dist/leaflet.css";
import { api } from "../../api";

interface GeoEvent {
  event_id: string;
  source: string;
  kind: string;
  title: string;
  lat: number;
  lon: number;
  severity: string;
  occurred_at: string;
}

const SEV_COLOR: Record<string, string> = {
  flash: "#ff3355",
  priority: "#ff9500",
  routine: "#ffd60a",
  info: "#38bdf8",
};

type TileProvider = "carto" | "esri";
const CARTO_KEY = (import.meta.env.VITE_CARTO_KEY as string | undefined) ?? "";
const PROVIDERS = {
  carto: {
    url: `https://{s}.basemaps.cartocdn.com/rastertiles/dark_all/{z}/{x}/{y}.png?key=${CARTO_KEY}`,
    options: { attribution: "&copy; OSM &copy; CARTO", subdomains: "abcd", maxZoom: 20 },
  },
  esri: {
    url: "https://server.arcgisonline.com/ArcGIS/rest/services/Canvas/World_Dark_Gray_Base/MapServer/tile/{z}/{y}/{x}",
    options: { attribution: "&copy; Esri", maxZoom: 16 },
  },
} as const;
const CHAIN: TileProvider[] = CARTO_KEY ? ["carto", "esri"] : ["esri"];
const PRIMARY = CHAIN[0];

// Frame↔map contract: the map always shows the INHABITED world fitted exactly
// into the card — longitude spans the full 359° so no repeated continent
// copies can ever render; only uninhabited polar/pacific fringes are trimmed;
// every continent (incl. East Asia) stays whole. Fractional zoomSnap 0.25 lets
// fitBounds fill any panel aspect precisely. Leaflet does not track container
// resizes by itself — invalidateSize() before every refit.
const WORLD: L.LatLngBoundsExpression = [[-58, -179], [76, 180]];

export default function MonitorMap({ refreshKey }: { refreshKey: number }) {
  const mapRef = useRef<L.Map | null>(null);
  const layerRef = useRef<L.LayerGroup | null>(null);
  const baseRef = useRef<L.Layer | null>(null);
  const divRef = useRef<HTMLDivElement>(null);
  const failCount = useRef(0);
  const tileProvider = useRef<TileProvider>(PRIMARY);
  const offlineGeo = useRef<GeoJSON.GeoJSON | null>(null);
  const eventsRef = useRef<GeoEvent[]>([]);
  const [selected, setSelected] = useState<GeoEvent | null>(null);

  function addTileLayer(map: L.Map, provider: TileProvider) {
    const p = PROVIDERS[provider];
    const layer = L.tileLayer(p.url, p.options);
    layer.on("tileerror", () => { failCount.current += 1; if (failCount.current >= 4) failover(); });
    layer.on("tileload", () => { failCount.current = 0; });
    baseRef.current = layer;
    layer.addTo(map);
    tileProvider.current = provider;
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
    addTileLayer(map, PRIMARY);
    const fit = () => {
      map.invalidateSize();
      if ((divRef.current?.clientWidth ?? 0) > 0) map.fitBounds(WORLD, { animate: false });
    };
    const raf = requestAnimationFrame(fit);
    const settle = setTimeout(fit, 300);
    const ro = new ResizeObserver(fit);
    ro.observe(divRef.current);
    const tm = setTimeout(() => { if (failCount.current > 0 && baseRef.current) failover(); }, 5000);
    return () => { clearTimeout(tm); clearTimeout(settle); cancelAnimationFrame(raf); ro.disconnect(); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function renderMarkers() {
    const lg = layerRef.current;
    if (!lg) return;
    lg.clearLayers();
    for (const ev of eventsRef.current) {
      const m = L.circleMarker([ev.lat, ev.lon], {
        radius: ev.severity === "flash" ? 6 : ev.severity === "priority" ? 5 : 3.5,
        color: SEV_COLOR[ev.severity] ?? "#38bdf8",
        weight: 1, fillOpacity: 0.7,
      });
      m.on("click", () => setSelected(ev));
      m.bindTooltip(ev.title, { direction: "top", offset: [0, -4] });
      lg.addLayer(m);
    }
  }

  useEffect(() => {
    let live = true;
    const from = new Date(Date.now() - 24 * 3600_000).toISOString();
    api<{ items: GeoEvent[] }>(`/api/v1/radar/events?from=${encodeURIComponent(from)}&limit=800`)
      .then((d) => { if (live) { eventsRef.current = d.items ?? []; renderMarkers(); } })
      .catch(() => {});
    return () => { live = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refreshKey]);

  return (
    <div className="hud-map-wrap">
      <div ref={divRef} className="absolute inset-0" />
      {selected && (
        <div
          className="absolute bottom-2 left-2 right-2 z-[500] rounded-md border p-2 text-[11px]"
          style={{ background: "rgba(4,8,14,.9)", borderColor: "var(--hud-edge)" }}
        >
          <div className="flex items-center gap-2">
            <span className="hud-dot" style={{ background: SEV_COLOR[selected.severity] ?? "#38bdf8" }} />
            <span className="hud-mono text-[9px] uppercase" style={{ color: "var(--hud-dim)" }}>
              {selected.kind} · {selected.source}
            </span>
            <span className="flex-1" />
            <button className="hud-btn" onClick={() => setSelected(null)}>×</button>
          </div>
          <div className="mt-1">{selected.title}</div>
        </div>
      )}
    </div>
  );
}
