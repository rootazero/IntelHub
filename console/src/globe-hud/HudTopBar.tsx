// HUD top bar (T11): brand mark, live location search, open-alert bell and a
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
import {
  useEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import { useLocation, useNavigate } from "react-router-dom";
import type { OverviewData } from "./useOverview";
import { HudStyleSwitcher } from "./HudStyleSwitcher";
import type { VisualEffectsHandle } from "../gev-visual/visual-effects";
import type { CameraOrientationHandle } from "../gev-visual/camera-orientation";
import type {
  LocationSearchHandle,
  SearchState,
} from "../gev-visual/location-search";
import type { ApiFetch } from "../gev-adapters/http";

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
  /** T-P6: visual-effects adapter handle — null hides the style switcher. */
  visualEffects?: VisualEffectsHandle | null;
  /** P7: camera orientation adapter — null hides the reset-north button. */
  camera?: CameraOrientationHandle | null;
  /** P7 location search. `undefined` = self-mount from apiFetch + viewer (the
   *  live GlobeV2 path); an explicit `null` = engine not ready, so the input
   *  renders disabled instead of pretending to search. Tests / external owners
   *  may inject a handle directly. */
  locationSearch?: LocationSearchHandle | null;
  /** P7: authenticated hub transport for the self-mounted location search. */
  apiFetch?: ApiFetch;
  /** P7: live viewer for the self-mounted location search. */
  viewer?: unknown;
}

export function HudTopBar({
  overview,
  visualEffects,
  camera,
  locationSearch,
  apiFetch,
  viewer,
}: HudTopBarProps) {
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

  const searchInputRef = useRef<HTMLInputElement>(null);
  const [selfSearch, setSelfSearch] = useState<LocationSearchHandle | null>(
    null,
  );
  const [query, setQuery] = useState("");
  const [searchState, setSearchState] = useState<SearchState>("idle");

  // Self-mount when no handle is injected (the live GlobeV2 path). The import
  // is lazy: the adapter pulls in the vendored engine (and Cesium), which the
  // injected-handle tests and the rest of the HUD chrome never need.
  useEffect(() => {
    if (locationSearch !== undefined) return;
    if (!apiFetch || !viewer) return;
    const input = searchInputRef.current;
    if (!input) return;
    let cancelled = false;
    let handle: LocationSearchHandle | null = null;
    import("../gev-visual/location-search")
      .then(({ mountLocationSearch }) => {
        if (cancelled) return;
        handle = mountLocationSearch(viewer, input, apiFetch);
        setSelfSearch(handle);
      })
      .catch((e) => console.warn("[HudTopBar] location search disabled:", e));
    return () => {
      cancelled = true;
      handle?.destroy();
      setSelfSearch(null);
    };
  }, [locationSearch, apiFetch, viewer]);

  // An injected handle wins; `undefined` means "use the self-mounted one".
  const search = locationSearch !== undefined ? locationSearch : selfSearch;

  // Mirror the adapter's state. missing/failed are transient feedback, not a
  // permanent label: they clear after 4 s or on the next keystroke.
  useEffect(() => {
    if (!search) {
      setSearchState("idle");
      return;
    }
    setSearchState(search.getState());
    return search.subscribe(setSearchState);
  }, [search]);

  useEffect(() => {
    if (searchState !== "missing" && searchState !== "failed") return;
    const timer = setTimeout(() => setSearchState("idle"), 4000);
    return () => clearTimeout(timer);
  }, [searchState]);

  const onSearchChange = (value: string) => {
    setQuery(value);
    if (searchState === "missing" || searchState === "failed") {
      setSearchState("idle");
    }
  };

  const onSearchKeyDown = (e: ReactKeyboardEvent<HTMLInputElement>) => {
    if (e.key !== "Enter") return;
    const q = query.trim();
    if (!q || !search) return;
    void Promise.resolve(search.run(q)).catch(() => {});
  };

  const searchStatusText =
    searchState === "searching"
      ? "搜索中…"
      : searchState === "missing"
        ? "未找到 / Not found"
        : searchState === "failed"
          ? "搜索失败 / Search failed"
          : null;

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
      {/* Live location search (P7). Disabled only while the engine is not
          ready (no handle yet) — honest degradation, not a placeholder. */}
      <input
        ref={searchInputRef}
        className="hud-bar-search"
        type="search"
        placeholder="地点搜索 / Location"
        aria-label="地点搜索 / Location search"
        title="地点搜索 / Location search"
        data-testid="hud-search-location"
        value={query}
        onChange={(e) => onSearchChange(e.target.value)}
        onKeyDown={onSearchKeyDown}
        disabled={!search}
      />
      {searchStatusText && (
        <span className="hud-search-status" data-testid="hud-search-status">
          {searchStatusText}
        </span>
      )}
      <span className="hud-bar-grow" />
      {camera && (
        <button
          type="button"
          className="hud-bar-back"
          onClick={() => camera.resetNorth()}
          title="回北 / Reset north"
          aria-label="Reset north"
          data-testid="hud-north-button"
        >
          ▲ N
        </button>
      )}
      <HudStyleSwitcher handle={visualEffects ?? null} />
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
