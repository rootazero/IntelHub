// HUD bottom status bar (T11): globe-collector health dots, enabled-layer
// count (the dataManager's enabled set) and the 24 h object count. Mounted by
// the page into HudFrame's [data-hud="bottom"] slot.
//
// Layer counts reuse T9's dataManager surface (getAll / isEffectivelyEnabled /
// subscribe — see HudLayerRail for the vendor contract verification) so the
// readout always agrees with the rail's checkboxes. The manager is a plain JS
// engine object (wildcard d.ts), so only this minimal structural type is used.
//
// Basemap label: the GEV globe is photoreal (Google 3D Tiles) when a Google key
// (or Cesium ion token) is configured, otherwise scene.js falls back to keyless
// `esri-imagery` (gev-engine/src/app/scene.js: initialStack). CARTO belongs to
// the P1 2D radar and is never this scene's basemap. The label is derived from
// key presence (not from the live styleManager, which the bootstrap does not
// expose) and carries a P3 tooltip so it is not mistaken for live state.
//
// Mouse lon/lat readout is a STATIC placeholder (P3): live wiring needs the
// Cesium viewer handle plumbed into the HUD plus a ScreenSpaceEventHandler
// lifecycle — same explicit-placeholder pattern as the P5 search box.
import { useEffect, useState } from "react";
import type { RailManager } from "./HudLayerRail";
import type { GlobeSourceHealth, OverviewData } from "./useOverview";

/** Globe-relevant collectors, in board order. These are the monitor source
 *  names written into `hub:monitor:health` (verified in
 *  hub-core/crates/hub-core/src/monitor/sources/*.rs). */
export const GLOBE_SOURCES = ["adsb", "celestrak", "usgs", "opensky"] as const;

/** Google key from the build (scripts/build-console.sh); empty on a keyless
 *  build, which is exactly the case scene.js resolves to `esri-imagery`. */
const GOOGLE_MAPS_KEY =
  (import.meta.env.VITE_GOOGLE_MAPS_KEY as string | undefined) ?? "";

/** Honest basemap label: the engine scene loads `photoreal` when a Google key /
 *  Cesium ion token is configured and `esri-imagery` otherwise
 *  (gev-engine/src/app/scene.js). Exported pure so both lanes are testable. */
export function basemapStyleName(googleKey?: string | null): string {
  return googleKey ? "GOOGLE PHOTOREAL" : "ESRI IMAGERY";
}

export type SourceTone = "ok" | "warn" | "bad";

/** state → dot tone. "warn" is the honest bucket for unproven sources: a cell
 *  that is missing / "absent" / "unknown" means no sweep has reported yet —
 *  claiming red would turn every fresh VM's first minutes into an alarm. */
export function sourceTone(source?: Pick<GlobeSourceHealth, "state">): SourceTone {
  const state = source?.state;
  if (state === "ok") return "ok";
  if (!state || state === "unknown" || state === "absent") return "warn";
  return "bad";
}

/** Only the manager slice the bar reads (a subset of T9's RailManager). */
export type BarManager = Pick<
  RailManager,
  "getAll" | "isEffectivelyEnabled" | "subscribe"
>;

export interface HudBottomBarProps {
  overview?: OverviewData | null;
  /** Engine data manager (getComponents().data.dataManager) — null until the
   *  data phase has booted. */
  manager?: BarManager | null;
  /** True when the last overview poll failed. */
  error?: boolean;
  /** Override for the build-time Google key (tests / keyless builds). */
  googleKey?: string;
}

export function HudBottomBar({
  overview,
  manager,
  error = false,
  googleKey,
}: HudBottomBarProps) {
  // The manager's lifecycle events are only a render trigger: getAll() /
  // isEffectivelyEnabled() are read live during render below, so the epoch
  // value itself is never needed. Bump-and-forget keeps the counts converging
  // while a toggle is still settling on its per-layer queue (T9 rail pattern).
  const [, setEpoch] = useState(0);
  useEffect(() => {
    if (!manager) return;
    return manager.subscribe((change) => {
      if (change?.layerId) setEpoch((value) => value + 1);
    });
  }, [manager]);

  const health = new Map<string, GlobeSourceHealth>(
    (overview?.radar?.monitor?.sources ?? []).map((source) => [
      source.name,
      source,
    ]),
  );

  let enabled = 0;
  let total = 0;
  if (manager) {
    const layers = manager
      .getAll()
      .filter((layer) => layer.showInTogglePanel !== false);
    total = layers.length;
    enabled = layers.filter((layer) =>
      manager.isEffectivelyEnabled(layer.id),
    ).length;
  }
  const geoEvents = overview?.radar?.geo_events_24h;
  const sourceOk = GLOBE_SOURCES.filter(
    (name) => health.get(name)?.state === "ok",
  ).length;
  const styleName = basemapStyleName(googleKey ?? GOOGLE_MAPS_KEY);

  return (
    <div className="hud-bar hud-bar-bottom" data-testid="hud-bottom-bar">
      {/* nav + aria-label: the collector names are links to the monitor page
          (route "/", App.tsx), so the group is a navigation landmark (no
          synthetic list roles). */}
      <nav className="hud-bar-sources" aria-label="采集器健康">
        {GLOBE_SOURCES.map((name) => {
          const source = health.get(name);
          const tone = sourceTone(source);
          const state = source?.state ?? "no-data";
          const lastNew = source?.last_new;
          return (
            <a
              key={name}
              className="hud-bar-src"
              href="/"
              data-testid={`hud-src-${name}`}
              title={`${name} · ${state}${
                typeof lastNew === "number" ? ` · 新增 ${lastNew}` : ""
              }`}
            >
              <i className={`hud-src-dot ${tone}`} aria-hidden />
              <span className="hud-bar-src-name">{name}</span>
            </a>
          );
        })}
      </nav>
      <span className="hud-bar-grow" />
      <span className="hud-bar-stat">
        源
        <b data-testid="hud-source-ok">
          {sourceOk}/{GLOBE_SOURCES.length}
        </b>
      </span>
      <span className="hud-bar-sep" aria-hidden />
      <span className="hud-bar-stat">
        图层
        <b data-testid="hud-layer-count">
          {manager ? `${enabled}/${total}` : "—/—"}
        </b>
      </span>
      <span className="hud-bar-sep" aria-hidden />
      <span className="hud-bar-stat">
        事件 24h
        <b data-testid="hud-object-count">
          {geoEvents == null ? "—" : geoEvents.toLocaleString("en-US")}
        </b>
      </span>
      <span className="hud-bar-sep" aria-hidden />
      {/* Static pointer readout — live ScreenSpaceEventHandler wiring is P3
          (same explicit-placeholder pattern as the P5 search box). */}
      <span
        className="hud-bar-stat hud-bar-coords"
        data-testid="hud-coords-p3"
        title="鼠标坐标（P3 接 Cesium ScreenSpaceEventHandler）"
        aria-label="鼠标坐标 lon — lat —（P3 待实现）"
      >
        <span className="hud-bar-coord">
          lon <b data-testid="hud-cursor-lon">—</b>
        </span>
        <span className="hud-bar-coord-sep" aria-hidden>
          /
        </span>
        <span className="hud-bar-coord">
          lat <b data-testid="hud-cursor-lat">—</b>
        </span>
      </span>
      <span className="hud-bar-sep" aria-hidden />
      <span
        className="hud-bar-stat"
        title="底图样式（P3 接引擎 styleManager）"
      >
        底图
        <b data-testid="hud-basemap-style">{styleName}</b>
      </span>
      {error && (
        <>
          <span className="hud-bar-sep" aria-hidden />
          <span className="hud-bar-stat bad" data-testid="hud-overview-state">
            overview 离线
          </span>
        </>
      )}
    </div>
  );
}
