// Shared MapLibre GeoJSON source + circle layer for geo_events on the map.
//
// Background — Radar.tsx and MonitorMap.tsx historically rendered each
// event as an individual maplibregl.Marker({ element: <div> }). At
// 1213+ events the DOM hit-testing cost + per-marker :hover transform +
// box-shadow keyframe (flash severity) made mouseover on the map visibly
// laggy in real browsers. This module replaces that with a single
// GeoJSON source + circle layer — one WebGL draw call per frame
// regardless of event count, hit-testing via GPU, no DOM per event.
//
// Usage:
//   const handle = attachEventsLayer(map, events, {
//     onSelect: (ev) => { setSelected(ev); },
//     kindFilter: (k) => k === currentKind || currentKind === '',
//     popup: true,
//     dim: (ev) => ev.source !== selectedSource,
//   });
//   ... later, when events change:
//   handle.setEvents(newEvents);
//   ... and on unmount:
//   handle.destroy();
//
// The layer survives map.setStyle() (e.g. CARTO → Esri → offline
// failover): a style.load listener re-adds the source and layer under
// their stable IDs once the new style is ready. setData() is idempotent
// against that lifecycle — it's a no-op if the source isn't there yet,
// and the post-load re-add carries the latest events payload.

import maplibregl, {
  Map as MlMap,
  type GeoJSONSource,
  type MapGeoJSONFeature,
} from "maplibre-gl";

// MapLibre's MapMouseEvent already carries typed `features?: MapGeoJSONFeature[]`
// when the event was registered against a layer — use it directly. The
// properties field is loosely typed (Record<string, unknown>) and we
// narrow at the call sites.
interface LayerClickEvent extends Omit<maplibregl.MapMouseEvent, "features"> {
  features?: MapGeoJSONFeature[];
}


/** Minimal event shape required by the layer (Radar/Monitor both have more). */
export interface MapEvent {
  event_id: string;
  source: string;
  kind: string;
  title: string;
  lat: number;
  lon: number;
  severity: string;
  occurred_at: string;
}

export interface AttachOptions {
  /** Called when the user clicks a circle. Receives the event whose
   *  event_id matches the clicked feature. */
  onSelect: (ev: MapEvent) => void;
  /** Optional filter — return false to exclude the event from the layer.
   *  Used by Radar's kind-chip filter and MonitorMap's source filter. */
  filter?: (ev: MapEvent) => boolean;
  /** Optional dim predicate — return true to dim the event (fade opacity).
   *  Used by MonitorMap when a source filter is active. */
  dim?: (ev: MapEvent) => boolean;
  /** Show a MapLibre Popup with kind + title on mouseenter. Default true. */
  popup?: boolean;
  /** Stable layer/source id prefix so multiple instances on the same map
   *  (none today, but defensive) don't collide. */
  idPrefix?: string;
  /** Optional kind→color map for the circle fill. Defaults to a hardcoded
   *  palette identical to the previous CSS markers so visuals match. */
  kindColors?: Record<string, string>;
}

export interface AttachHandle {
  setEvents(next: MapEvent[]): void;
  destroy(): void;
}

const DEFAULT_KIND_COLORS: Record<string, string> = {
  // Same palette the previous .radar-marker used (matched via kindColor()
  // helper in kindmeta). Kept here as a static fallback so this module
  // has zero import dependencies on the rest of the console.
  flight: "#5eead4",
  cyber: "#a78bfa",
  news: "#f472b6",
  quake: "#fb923c",
  financial: "#facc15",
  ais: "#34d399",
  satellite: "#60a5fa",
  weather: "#22d3ee",
  military: "#f87171",
  infra: "#94a3b8",
  default: "#8a8f98",
};

const SEV_RADIUS: Record<string, number> = {
  flash: 8,
  priority: 7,
  routine: 6,
  info: 5,
};

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function buildFeatureCollection(events: MapEvent[], filter?: (e: MapEvent) => boolean) {
  return {
    type: "FeatureCollection" as const,
    features: events
      .filter((e) => !filter || filter(e))
      .map((e) => ({
        type: "Feature" as const,
        id: undefined as unknown as number, // string IDs confuse feature-state; use props.event_id
        geometry: {
          type: "Point" as const,
          coordinates: [e.lon, e.lat],
        },
        properties: {
          event_id: e.event_id,
          kind: e.kind,
          title: e.title,
          severity: e.severity,
          source: e.source,
          occurred_at: e.occurred_at,
        },
      })),
  };
}

function paintExpressions(
  colors: Record<string, string>,
  dimExpr: unknown[] | null,
) {
  // circle-color: by kind, fallback to default
  const kindMatch: unknown[] = ["match", ["get", "kind"]];
  for (const [k, c] of Object.entries(colors)) kindMatch.push(k, c);
  kindMatch.push(colors.default ?? "#8a8f98");

  // circle-radius: by severity, fallback 5
  const sevMatch: unknown[] = ["match", ["get", "severity"]];
  for (const [k, r] of Object.entries(SEV_RADIUS)) sevMatch.push(k, r);
  sevMatch.push(5);

  // circle-opacity: 0.55 for info severity; if dimExpr is provided,
  // fade non-matching events down further (used by MonitorMap's source
  // filter).
  let opacity: unknown[];
  if (dimExpr) {
    opacity = ["case", dimExpr, 0.18, ["match", ["get", "severity"], "info", 0.55, 0.85]];
  } else {
    opacity = ["match", ["get", "severity"], "info", 0.55, 0.85];
  }

  // circle-stroke-color matches kind color (slightly muted by virtue of
  // being a stroke — same palette as the old DOM marker border).
  const strokeMatch: unknown[] = ["match", ["get", "kind"]];
  for (const [k, c] of Object.entries(colors)) strokeMatch.push(k, c);
  strokeMatch.push(colors.default ?? "#8a8f98");

  return {
    "circle-radius": sevMatch as never,
    "circle-color": kindMatch as never,
    "circle-opacity": opacity as never,
    "circle-stroke-width": 2,
    "circle-stroke-color": strokeMatch as never,
  };
}

export function attachEventsLayer(
  map: MlMap,
  initialEvents: MapEvent[],
  opts: AttachOptions,
): AttachHandle {
  const prefix = opts.idPrefix ?? "events";
  const sourceId = `${prefix}-src`;
  const layerId = `${prefix}-layer`;
  const colors = { ...DEFAULT_KIND_COLORS, ...(opts.kindColors ?? {}) };
  const showPopup = opts.popup !== false;

  let currentEvents = initialEvents;
  let popup: maplibregl.Popup | null = null;
  let destroyed = false;

  // ----- dim expression (rebuilt on setEvents when opts.dim is set) -----
  // MapLibre paint expressions can't reference external JS state, so we
  // pre-compute the set of dimmed sources each time setEvents() is
  // called and emit an `in [...literal...]` expression against it.
  let dimmedSources = new Set<string>();
  function rebuildDim() {
    if (!opts.dim) return;
    const next = new Set<string>();
    for (const e of currentEvents) if (opts.dim(e)) next.add(e.source);
    if (setsEqual(next, dimmedSources)) return;
    dimmedSources = next;
    const layer = map.getLayer(layerId);
    if (!layer) return;
    const expr: unknown[] = dimmedSources.size === 0
      ? ["match", ["get", "severity"], "info", 0.55, 0.85]
      : [
          "case",
          ["in", ["get", "source"], ["literal", Array.from(dimmedSources)]],
          0.18,
          ["match", ["get", "severity"], "info", 0.55, 0.85],
        ];
    map.setPaintProperty(layerId, "circle-opacity", expr as never);
  }

  // ----- add or update source + layer -----
  function ensureLayer() {
    if (destroyed) return;
    let source = map.getSource(sourceId) as GeoJSONSource | undefined;
    if (!source) {
      map.addSource(sourceId, {
        type: "geojson",
        data: buildFeatureCollection(currentEvents, opts.filter),
      });
      source = map.getSource(sourceId) as GeoJSONSource | undefined;
    } else {
      source.setData(buildFeatureCollection(currentEvents, opts.filter));
    }
    if (!map.getLayer(layerId)) {
      map.addLayer({
        id: layerId,
        type: "circle",
        source: sourceId,
        paint: paintExpressions(colors, null),
      });
      rebuildDim();
      // Click → onSelect. We resolve the event from currentEvents by
      // event_id (props.event_id) so callers always see the latest
      // snapshot, not a stale captured reference.
      map.on("click", layerId, onClick);
      // Cursor + popup on hover.
      map.on("mouseenter", layerId, onEnter);
      map.on("mouseleave", layerId, onLeave);
    }
  }

  function onClick(e: LayerClickEvent) {
    const f = e.features?.[0];
    if (!f) return;
    const id = (f.properties as { event_id?: string } | null)?.event_id;
    if (!id) return;
    const ev = currentEvents.find((x) => x.event_id === id);
    if (ev) opts.onSelect(ev);
  }
  function onEnter(e: LayerClickEvent) {
    map.getCanvas().style.cursor = "pointer";
    if (!showPopup) return;
    const f = e.features?.[0];
    if (!f) return;
    const geom = f.geometry;
    if (!geom || geom.type !== "Point") return;
    const coords = geom.coordinates as unknown;
    if (!Array.isArray(coords) || coords.length < 2 || typeof coords[0] !== "number") return;
    const props = f.properties as { kind?: string; title?: string } | null;
    if (popup) popup.remove();
    popup = new maplibregl.Popup({
      closeButton: false,
      closeOnClick: false,
      offset: 10,
      className: "events-tooltip",
      maxWidth: "240px",
    })
      .setLngLat(coords as [number, number])
      .setHTML(
        `<div class="events-tooltip-kind">${escapeHtml(props?.kind ?? "")}</div>` +
          `<div class="events-tooltip-title">${escapeHtml(props?.title ?? "")}</div>`,
      )
      .addTo(map);
  }
  function onLeave() {
    map.getCanvas().style.cursor = "";
    if (popup) {
      popup.remove();
      popup = null;
    }
  }

  // ----- style.load: re-add layer if setStyle() wiped it -----
  // MapLibre's constructor is synchronous but the initial style fetch
  // is async — addSource/addLayer before 'style.load' fires throws
  // "Style is not done loading". Defer until the event fires; if the
  // event already fired before this registration (race against the
  // map constructor), isStyleLoaded() lets us attach immediately.
  const onStyleLoad = () => ensureLayer();
  map.on("style.load", onStyleLoad);
  if (map.isStyleLoaded()) {
    ensureLayer();
  }
  // else: style.load handler will pick it up. setEvents() also short-
  // circuits when the source isn't present yet, so callers can push
  // data before style load without erroring.

  return {
    setEvents(next) {
      currentEvents = next;
      const source = map.getSource(sourceId) as GeoJSONSource | undefined;
      if (source) {
        source.setData(buildFeatureCollection(next, opts.filter));
        rebuildDim();
      }
      // else: style still loading. The style.load handler will call
      // ensureLayer() which reads currentEvents at that point. We must
      // NOT call ensureLayer() here — addSource before style.load
      // throws "Style is not done loading".
    },
    destroy() {
      destroyed = true;
      map.off("style.load", onStyleLoad);
      map.off("click", layerId, onClick);
      map.off("mouseenter", layerId, onEnter);
      map.off("mouseleave", layerId, onLeave);
      if (popup) {
        popup.remove();
        popup = null;
      }
      if (map.getLayer(layerId)) map.removeLayer(layerId);
      if (map.getSource(sourceId)) map.removeSource(sourceId);
    },
  };
}

function setsEqual<T>(a: Set<T>, b: Set<T>): boolean {
  if (a.size !== b.size) return false;
  for (const v of a) if (!b.has(v)) return false;
  return true;
}

