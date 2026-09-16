// Top-right region-jump + zoom + reset, shared between Radar and
// MonitorMap. Behavior is one source of truth (useMapView); CSS theming
// differs via the `className` prop — caller picks `.hud-map-ctrl` (dark
// glass on the command deck) or `.radar-map-ctrl` (panel-style on Radar).
//
// CSS rules live in their respective stylesheets (hud.css / index.css)
// so the visual theme stays per-page; only behavior is shared.
import { useT } from "../i18n";
import { REGION_KEYS } from "../mapControls";
import { useMapView } from "../useMapView";

export function MapControls({ className }: { className: string }) {
  const { t } = useT();
  const { region, flyToRegion, zoomIn, zoomOut, reset } = useMapView();
  return (
    <div className={className}>
      <div className="map-ctrl-row">
        {REGION_KEYS.map((r) => (
          <button
            key={r}
            className={`map-ctrl-btn ${region === r ? "on" : ""}`}
            onClick={() => flyToRegion(r)}
            title={t(`hud.region.${r}`)}
          >
            {t(`hud.region.${r}`)}
          </button>
        ))}
      </div>
      <div className="map-ctrl-row">
        <button className="map-ctrl-btn icon" title={t("hud.zoomIn")} onClick={zoomIn}>+</button>
        <button className="map-ctrl-btn icon" title={t("hud.zoomOut")} onClick={zoomOut}>−</button>
        <button className="map-ctrl-btn icon" title={t("hud.zoomReset")} onClick={reset}>⌂</button>
      </div>
    </div>
  );
}