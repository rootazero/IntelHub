import { useT } from "../../i18n";
import { TimeAgo } from "../../ui";

export type DeltaDir = "up" | "down" | "mixed" | "flat" | "new_source";

/** Topbar: sources badge, delta direction pill, last sweep, open alerts, VISUALS toggle. */
export default function TopStrip({
  sourcesOk,
  sourcesTotal,
  direction,
  lastSweep,
  openAlerts,
  visuals,
  onToggleVisuals,
}: {
  sourcesOk: number;
  sourcesTotal: number;
  direction: DeltaDir;
  lastSweep: string | null;
  openAlerts: number;
  visuals: "full" | "lite";
  onToggleVisuals: () => void;
}) {
  const { t } = useT();
  const dirClass =
    direction === "up" ? "hud-pill-up" :
    direction === "down" ? "hud-pill-down" :
    direction === "mixed" || direction === "new_source" ? "hud-pill-mixed" : "hud-pill-flat";
  const dirLabel =
    direction === "up" ? `▲ ${t("hud.dirUp")}` :
    direction === "down" ? `▼ ${t("hud.dirDown")}` :
    direction === "mixed" ? `◆ ${t("hud.dirMixed")}` :
    direction === "new_source" ? `NEW` : `= ${t("hud.dirFlat")}`;
  const allOk = sourcesTotal > 0 && sourcesOk === sourcesTotal;

  return (
    <div className="hud-topstrip">
      <span className="hud-mono text-[12px] font-bold tracking-[0.2em]" style={{ color: "var(--hud-accent)" }}>
        INTELHUB
      </span>
      <span className="text-[9px] uppercase tracking-[0.3em]" style={{ color: "var(--hud-dim)" }}>
        {t("hud.deck")}
      </span>
      <span className={`hud-pill ${allOk ? "hud-pill-down" : "hud-pill-up"}`}>
        <span className={`hud-dot ${allOk ? "hud-dot-ok hud-pulse" : "hud-dot-bad hud-pulse"}`} />
        {t("hud.sources")} {sourcesOk}/{sourcesTotal}
      </span>
      <span className={`hud-pill ${dirClass}`}>{dirLabel}</span>
      {lastSweep && (
        <span className="hud-pill hud-pill-flat">
          {t("hud.lastSweep")} <TimeAgo ts={lastSweep} />
        </span>
      )}
      <span className="flex-1" />
      {openAlerts > 0 && (
        <a href="#/alerts" className="hud-pill hud-pill-up hud-pulse" style={{ textDecoration: "none" }}>
          ⚠ {openAlerts} {t("hud.openAlerts")}
        </a>
      )}
      <button className={`hud-btn ${visuals === "full" ? "on" : ""}`} onClick={onToggleVisuals}>
        {t("hud.visuals")} {visuals === "full" ? t("hud.full") : t("hud.lite")}
      </button>
    </div>
  );
}
