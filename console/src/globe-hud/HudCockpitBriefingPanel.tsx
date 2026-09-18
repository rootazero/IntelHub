// GEV P9 cockpit briefing panel — dual tab (Weather / Summary) + summary-bullet
// auto-rotation at the vendor cadence (COCKPIT_BRIEF_ROTATE_MS) + hover pause.
//
// The T3 briefing adapter owns fetch + normalization (it does NOT call the
// vendor's DOM-coupled renderRegionalBrief); this panel renders the normalized
// Briefing view model directly as HUD DOM (T3 concern #3). Weather 503 →
// grey banner, no retry storm (D3).
import { useEffect, useRef, useState } from "react";
import { COCKPIT_BRIEF_ROTATE_MS } from "gev-engine/src/ui/cockpitPresentation.js";
import type {
  Briefing,
  BriefingHandle,
} from "../gev-visual/cockpit/briefing-mount";
import type { CockpitTrackedInfo } from "../gev-visual/cockpit/instruments-mount";

type Tab = "weather" | "summary";

export interface HudCockpitBriefingPanelProps {
  briefing: BriefingHandle | null;
  /** flights.getTrackedInfo() seam — supplies lat/lon + the entity id. */
  getTrackedInfo?: () => CockpitTrackedInfo | null;
  /** Store-backed pause flag (hover pauses the rotation). */
  paused?: boolean;
  onPause?: () => void;
  onResume?: () => void;
}

function entityIdOf(info: CockpitTrackedInfo | null): string {
  const key = info?.callsign || info?.icao24 || "unknown";
  return `flight:${key}`;
}

export function HudCockpitBriefingPanel({
  briefing,
  getTrackedInfo,
  paused = false,
  onPause,
  onResume,
}: HudCockpitBriefingPanelProps) {
  const [tab, setTab] = useState<Tab>("weather");
  const [data, setData] = useState<Briefing | null>(null);
  const [index, setIndex] = useState(0);
  const fetchSeq = useRef(0);

  // Fetch on mount and whenever the tracked aircraft's identity/position
  // changes. The AbortController + seq guard make it StrictMode-safe and stop
  // a stale response from clobbering a newer one.
  useEffect(() => {
    if (!briefing) return;
    const info = getTrackedInfo?.() ?? null;
    const seq = ++fetchSeq.current;
    let aborted = false;
    const controller = new AbortController();

    if (
      !info ||
      info.latitude == null ||
      info.longitude == null ||
      !Number.isFinite(info.latitude) ||
      !Number.isFinite(info.longitude)
    ) {
      setData(null);
      setIndex(0);
      return () => {
        aborted = true;
      };
    }

    void briefing
      .fetch(info.latitude, info.longitude, entityIdOf(info), {
        signal: controller.signal,
      })
      .then((b) => {
        if (!aborted && seq === fetchSeq.current) {
          setData(b);
          setIndex(briefing.index());
        }
      })
      .catch((error) => {
        if (!aborted && (error as { name?: string })?.name !== "AbortError") {
          console.warn("[hud-cockpit] briefing fetch failed:", error);
        }
      });

    return () => {
      aborted = true;
      controller.abort();
    };
  }, [briefing, getTrackedInfo]);

  // Summary-bullet auto-rotation at the vendor cadence. `data` in deps re-arms
  // the timer once the first fetch lands (total() reads the fetched bullets).
  useEffect(() => {
    if (!briefing || paused || briefing.total() === 0) return;
    const timer = setInterval(() => {
      briefing.next();
      setIndex(briefing.index());
    }, COCKPIT_BRIEF_ROTATE_MS);
    return () => clearInterval(timer);
  }, [briefing, paused, data]);

  const onNext = () => {
    briefing?.next();
    setIndex(briefing?.index() ?? 0);
  };
  const onPrev = () => {
    briefing?.prev();
    setIndex(briefing?.index() ?? 0);
  };

  const weather = data?.weather;
  const summary = data?.summary;
  const bullets = summary?.bullets ?? [];
  const currentBullet = bullets[index] ?? null;
  const allDegraded =
    (weather?.degraded ?? true) && (summary?.degraded ?? true);

  return (
    <div
      className="hud-cockpit-briefing"
      data-testid="hud-cockpit-briefing"
      onMouseEnter={onPause}
      onMouseLeave={onResume}
    >
      <div className="hud-cockpit-briefing-tabs" role="tablist">
        <button
          type="button"
          role="tab"
          aria-selected={tab === "weather"}
          data-testid="hud-cockpit-tab-weather"
          className={`hud-cockpit-tab${tab === "weather" ? " active" : ""}`}
          onClick={() => setTab("weather")}
        >
          Weather
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={tab === "summary"}
          data-testid="hud-cockpit-tab-summary"
          className={`hud-cockpit-tab${tab === "summary" ? " active" : ""}`}
          onClick={() => setTab("summary")}
        >
          Summary
        </button>
      </div>

      {tab === "weather" ? (
        <div className="hud-cockpit-briefing-body">
          {weather?.degraded || !weather ? (
            <div className="hud-cockpit-briefing-banner">
              简报暂不可用 — 天气数据源离线
            </div>
          ) : (
            <div className="hud-cockpit-weather-grid">
              {weather.metrics.map((m) => (
                <div className="hud-cockpit-weather-cell" key={m.key}>
                  <span className="hud-cockpit-weather-label">{m.label}</span>
                  <span className="hud-cockpit-weather-value">
                    {m.value ?? "—"}
                  </span>
                  {m.detail && (
                    <span className="hud-cockpit-weather-detail">{m.detail}</span>
                  )}
                </div>
              ))}
            </div>
          )}
        </div>
      ) : (
        <div className="hud-cockpit-briefing-body">
          {allDegraded || summary?.empty || bullets.length === 0 ? (
            <div className="hud-cockpit-briefing-banner">
              简报暂不可用 — 无聚合情报源
            </div>
          ) : (
            <div className="hud-cockpit-summary">
              <div className="hud-cockpit-summary-bullet">
                {currentBullet?.text ?? ""}
              </div>
              <div className="hud-cockpit-summary-meta">
                {currentBullet?.sourceUrl ? (
                  <a
                    href={currentBullet.sourceUrl}
                    target="_blank"
                    rel="noreferrer"
                    className="hud-cockpit-summary-source"
                  >
                    来源
                  </a>
                ) : null}
                <span className="hud-cockpit-summary-count">
                  {bullets.length > 0 ? `${index + 1}/${bullets.length}` : ""}
                </span>
              </div>
            </div>
          )}
        </div>
      )}

      <div className="hud-cockpit-briefing-nav">
        <button
          type="button"
          onClick={onPrev}
          aria-label="上一页 / Previous"
          className="hud-cockpit-nav-btn"
        >
          ←
        </button>
        <button
          type="button"
          onClick={onNext}
          aria-label="下一页 / Next"
          className="hud-cockpit-nav-btn"
        >
          →
        </button>
      </div>
    </div>
  );
}
