// Radar (§26) — global signal map fed by hub geo_events (native monitor sweeps).
// Dual basemap: CARTO dark tiles by default; on tile failure burst or first
// load timeout, falls back to the bundled offline world vector layer.
import { useEffect, useRef, useState } from "react";
import L from "leaflet";
import "leaflet/dist/leaflet.css";
import { api, streamEvents } from "../api";
import { useEnum, useT } from "../i18n";
import { KINDS, kindColor, sevRadius } from "../kindmeta";

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

type TilesMode = "stadia" | "carto" | "esri" | "offline";
type TileProvider = "stadia" | "carto" | "esri";

// Basemap chain. Two optional providers both supported:
//   - Stadia Maps (alidade_smooth_dark): primary choice when a key is
//     available. Designed as a data-overlay canvas (true black,
//     designed for OSINT use). Free tier: 200K credits/month,
//     non-commercial use permitted (stadiamaps.com/sign-up).
//   - CARTO dark_all: legacy primary. 5M tiles/month free for
//     non-commercial, requires key at carto.com/fascap/apikey.
// Without either key the chain starts at Esri (which is grey, not true
// black, but always available without auth).
// A burst of ≥4 tile errors (or no successful tile in the first 5s)
// advances one tier; the last tier is the bundled offline GeoJSON.
const STADIA_KEY = (import.meta.env.VITE_STADIA_KEY as string | undefined) ?? "";
const CARTO_KEY = (import.meta.env.VITE_CARTO_KEY as string | undefined) ?? "";
const PROVIDERS = {
  stadia: {
    url: `https://tiles.stadiamaps.com/tiles/alidade_smooth_dark/{z}/{x}/{y}{r}.png?api_key=${STADIA_KEY}`,
    options: {
      attribution:
        '&copy; <a href="https://stadiamaps.com/">Stadia Maps</a> &copy; <a href="https://www.openstreetmap.org/copyright">OSM</a>',
      subdomains: "abcd",
      maxZoom: 20,
    },
  },
  carto: {
    url: `https://{s}.basemaps.cartocdn.com/rastertiles/dark_all/{z}/{x}/{y}.png?key=${CARTO_KEY}`,
    options: {
      attribution:
        '&copy; <a href="https://www.openstreetmap.org/copyright">OSM</a> &copy; <a href="https://carto.com/attributions">CARTO</a>',
      subdomains: "abcd",
      maxZoom: 20,
    },
  },
  esri: {
    url: "https://server.arcgisonline.com/ArcGIS/rest/services/Canvas/World_Dark_Gray_Base/MapServer/tile/{z}/{y}/{x}",
    options: {
      attribution: '&copy; <a href="https://www.esri.com/">Esri</a> &mdash; Esri, DeLorme, NAVTEQ',
      maxZoom: 16,
    },
  },
} as const;
// Order: stadia if keyed, else carto if keyed, else esri.
const CHAIN: TileProvider[] =
  STADIA_KEY ? ["stadia", "esri"]
  : CARTO_KEY ? ["carto", "esri"]
  : ["esri"];
const PRIMARY = CHAIN[0];

export default function Radar() {
  const { t } = useT();
  const en = useEnum();
  const mapRef = useRef<L.Map | null>(null);
  const layerRef = useRef<L.LayerGroup | null>(null);
  const baseRef = useRef<L.Layer | null>(null);
  const divRef = useRef<HTMLDivElement>(null);
  const [events, setEvents] = useState<GeoEvent[]>([]);
  const [selected, setSelected] = useState<GeoEvent | null>(null);
  const [tiles, setTiles] = useState<TilesMode>(PRIMARY);
  const [window_, setWindow_] = useState("24h");
  const [sev, setSev] = useState("");
  const [kind, setKind] = useState("");
  const [creating, setCreating] = useState(false);
  const failCount = useRef(0);
  const tileProvider = useRef<TileProvider>(PRIMARY);
  const offlineGeo = useRef<GeoJSON.GeoJSON | null>(null);

  // ---- map init (once) ----
  useEffect(() => {
    if (!divRef.current || mapRef.current) return;
    const map = L.map(divRef.current, {
      center: [25, 10],
      zoom: 2,
      minZoom: 2,
      maxZoom: 10,
      worldCopyJump: true,
      attributionControl: true,
      zoomControl: true,
    });
    mapRef.current = map;
    layerRef.current = L.layerGroup().addTo(map);
    addTileLayer(map, PRIMARY);

    // first-load timeout: if tiles are erroring after 5s, advance one tier
    const t = setTimeout(() => {
      if (failCount.current > 0 && baseRef.current) failover();
    }, 5000);
    return () => clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function addTileLayer(map: L.Map, provider: TileProvider) {
    const p = PROVIDERS[provider];
    const layer = L.tileLayer(p.url, p.options);
    layer.on("tileerror", () => {
      failCount.current += 1;
      if (failCount.current >= 4) failover();
    });
    layer.on("tileload", () => {
      failCount.current = 0;
    });
    baseRef.current = layer;
    layer.addTo(map);
    tileProvider.current = provider;
    setTiles(provider);
  }

  // advance the basemap chain along CHAIN; last resort is offline GeoJSON
  function failover() {
    const map = mapRef.current;
    if (!map) return;
    const idx = CHAIN.indexOf(tileProvider.current);
    const next = CHAIN[idx + 1] as TileProvider | undefined;
    if (next) {
      if (baseRef.current) {
        map.removeLayer(baseRef.current);
        baseRef.current = null;
      }
      failCount.current = 0;
      addTileLayer(map, next);
    } else {
      void goOffline();
    }
  }

  async function goOffline() {
    const map = mapRef.current;
    if (!map) return;
    if (baseRef.current) {
      map.removeLayer(baseRef.current);
      baseRef.current = null;
    }
    if (!offlineGeo.current) {
      try {
        const r = await fetch("/world-110m.geo.json");
        offlineGeo.current = (await r.json()) as GeoJSON.GeoJSON;
      } catch {
        return;
      }
    }
    baseRef.current = L.geoJSON(offlineGeo.current, {
      style: {
        color: "#2a3b4d",
        weight: 0.8,
        fillColor: "#101820",
        fillOpacity: 0.85,
      },
    });
    baseRef.current.addTo(map);
    setTiles("offline");
  }

  // manual retry: back to the top of the chain
  function goOnline() {
    const map = mapRef.current;
    if (!map) return;
    if (baseRef.current) map.removeLayer(baseRef.current);
    failCount.current = 0;
    addTileLayer(map, PRIMARY);
  }

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
    const lg = layerRef.current;
    if (!lg) return;
    lg.clearLayers();
    for (const e of visible) {
      const m = L.circleMarker([e.lat, e.lon], {
        radius: sevRadius(e.severity),
        color: kindColor(e.kind),
        weight: e.severity === "flash" ? 2 : 1,
        fillColor: kindColor(e.kind),
        fillOpacity: e.severity === "info" ? 0.35 : 0.6,
      });
      m.on("click", () => setSelected(e));
      m.bindTooltip(`${en("kind", e.kind)} · ${e.title}`, { direction: "top" });
      m.addTo(lg);
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
          {!STADIA_KEY && !CARTO_KEY && (
            <span
              className="rounded bg-rose-500/15 px-1.5 py-0.5 mono text-[10px] text-rose-400"
              title="Dark basemap key missing from build — chain starts at Esri fallback (grey not black). Sign up for free at stadiamaps.com (preferred, non-commercial OSINT use) or carto.com/basemaps/apikey (5M tiles/mo), then rebuild the console with VITE_STADIA_KEY or VITE_CARTO_KEY set."
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
            <button className="rounded border border-edge px-1.5 py-0.5 text-[10px] hover:border-accent" onClick={goOnline}>
              {t("radar.retryOnline")}
            </button>
          )}
        </span>
      </div>
      <div className="flex min-h-0 flex-1">
        <div ref={divRef} className="min-w-0 flex-1" style={{ background: "#0a0e14" }} />
        {selected && (
          <aside className="w-80 shrink-0 overflow-y-auto border-l border-edge bg-panel p-3 text-xs">
            <div className="mb-2 flex items-center justify-between">
              <span className="font-semibold" style={{ color: SEV_COLOR[selected.severity] ?? SEV_COLOR.info }}>
                {en("severity", selected.severity).toUpperCase()} · {en("kind", selected.kind)}
              </span>
              <button className="text-dim hover:text-ink" onClick={() => setSelected(null)}>✕</button>
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
