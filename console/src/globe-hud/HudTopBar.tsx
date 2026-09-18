// HUD top bar (T11): brand mark, P5 search placeholder, open-alert bell and a
// 1 s UTC clock. Mounted by the page into HudFrame's [data-hud="top"] slot.
//
// Styling follows the P1 HUD token family carried by globe-hud/hud.css
// (rgba(3,5,9,.55) glass / #9aa4b2 dim / #e6edf3 ink / #ff9500 accent) — no
// CSS variables, no new palette.
//
// Navigation uses a plain <a href> (T10 precedent): the HUD chrome is
// engine-side, router-free, and a strip navigation is a full document load
// either way.
//
// EXCEPTION: the Back button uses react-router's navigate() instead of <a href>
// because Globe is fullscreen and the sidebar is hidden — this is the user's
// only in-app nav affordance, so it must not trigger a full page reload.
import { useEffect, useState } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import type { OverviewData } from "./useOverview";

/** HH:MM:SS in UTC — the clock never reads local time. */
export function formatUtcClock(date: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(date.getUTCHours())}:${pad(date.getUTCMinutes())}:${pad(
    date.getUTCSeconds(),
  )}`;
}

/** 1 s UTC ticker (brief: 时钟 1s 刻度). */
export function useUtcClock(intervalMs = 1000): Date {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const timer = setInterval(() => setNow(new Date()), intervalMs);
    return () => clearInterval(timer);
  }, [intervalMs]);
  return now;
}

// Hand-drawn globe mark: console/public ships no hub-mark.svg (verified — only
// world-110m.geo.json; the brief's "可复用" turns out impossible), and the HUD
// must not depend on an asset that does not exist. Accent colors only.
function HubMark() {
  return (
    <svg
      className="hud-bar-mark"
      viewBox="0 0 24 24"
      width="18"
      height="18"
      aria-hidden
      focusable="false"
    >
      <circle cx="12" cy="12" r="9" fill="none" stroke="#ff9500" strokeWidth="1.4" />
      <ellipse cx="12" cy="12" rx="4" ry="9" fill="none" stroke="#ff9500" strokeWidth="1" />
      <path d="M3.5 9h17M3.5 15h17" stroke="#9aa4b2" strokeWidth="0.9" />
    </svg>
  );
}

export interface HudTopBarProps {
  /** Snapshot from GET /api/v1/overview (useOverview) — null while loading. */
  overview?: OverviewData | null;
}

export function HudTopBar({ overview }: HudTopBarProps) {
  const now = useUtcClock();
  const openAlerts = overview?.alerts?.open;
  const clock = formatUtcClock(now);
  // Globe is fullscreen — sidebar hidden by design. The Back button at the
  // top-left is the user's only in-app nav affordance; location.key ===
  // "default" means the user landed here directly (typed URL / bookmark /
  // link from outside), so navigate(-1) would dump them off the app.
  // Fallback to "/" (Command Deck) keeps them inside the console.
  const navigate = useNavigate();
  const location = useLocation();
  const onBack = () => {
    if (location.key !== "default") navigate(-1);
    else navigate("/");
  };

  return (
    <div className="hud-bar hud-bar-top" data-testid="hud-top-bar">
      <button
        type="button"
        className="hud-bar-back"
        onClick={onBack}
        title="返回 / Back"
        aria-label="Back"
        data-testid="hud-back-button"
      >
        ← Back
      </button>
      <HubMark />
      <span className="hud-bar-brand">INTELHUB</span>
      <span className="hud-bar-sub">GEV · 全球态势感知</span>
      <span className="hud-bar-sep" aria-hidden />
      {/* P5 placeholder: rendered disabled on purpose so the control is
          visible and honest instead of a fake input that silently no-ops. */}
      <input
        className="hud-bar-search"
        type="search"
        placeholder="搜索目标 / 情报 (P5)"
        aria-label="全局搜索（P5 待实现）"
        title="全局搜索 P5 待实现"
        data-testid="hud-search-p5"
        disabled
      />
      <span className="hud-bar-grow" />
      <a
        className={`hud-bar-alert${openAlerts ? " hot" : ""}`}
        href="/alerts"
        title={openAlerts == null ? "告警中心" : `未处理告警 ${openAlerts}`}
        data-testid="hud-alert-bell"
      >
        <span aria-hidden>🔔</span>
        <span className="hud-bar-alert-count" data-testid="hud-alert-count">
          {openAlerts ?? "—"}
        </span>
      </a>
      <span
        className="hud-bar-clock"
        data-testid="hud-utc-clock"
        title={`UTC ${now.toISOString()}`}
      >
        {clock} UTC
      </span>
    </div>
  );
}
