// HudGlobeLoadingOverlay — full-screen "Initializing global view" placeholder.
//
// Why this exists: adsbx sweep (8 US hubs) takes ~200 s for a full cycle, and
// on first page load after hub-core restart the cache is empty until the
// first sweep lands. During that window the globe renders empty flight
// space, and operators misread the partial population (Florida cluster
// arrives first because ATL hub is index 0) as a data/coverage bug. The P20
// CA-coverage PR fixed the actual missing-data root cause; this overlay
// disambiguates the *warming-up* window from a real coverage gap.
//
// Visibility contract:
//   * Show  when (a) the flights layer is enabled, AND
//             (b) the most recent aircraft snapshot has < READY_THRESHOLD
//                 rows OR has not yet been fetched, AND
//             (c) we are within TIMEOUT_MS of mount.
//   * Hide  when any of those three fails. The timeout guarantees the
//           overlay never strands the page if upstream is degraded-by-design
//           (e.g., every adsbx hub returns 429 for the first cycle).
//
// Data source: a 1 Hz poll of /api/v1/globe/aircraft, reusing the same
// bearer-auth transport the aircraft layer reads from. We re-read instead of
// plumbing through the dataManager so the overlay stays a pure-React concern
// (no engine seam) and so a snapshot blip in the vendor flights ingestion
// (P3 lenient-mock class of bug) doesn't leave the overlay stuck on.
//
// Out of scope: per-hub progress (we only count total aircraft, not which
// hubs have answered). A progress bar would suggest a level of fidelity the
// 1 Hz REST poll can't deliver without a new hub-core endpoint.

import { useEffect, useState } from "react";
import { api, ApiError } from "../api";

const POLL_MS = 1_000;
/** Aircraft count above which we declare the global view "ready". 100 ≈
 *  roughly one major US metro's worth — well under the post-sweep total
 *  (~400+) but enough to disprove the empty-globe perception. */
const READY_THRESHOLD = 100;
/** Upper bound on the placeholder's lifetime, regardless of upstream state.
 *  Past this point we assume the data path is broken or the operator
 *  intentionally disabled the layer, and we stop blocking the view. */
const TIMEOUT_MS = 30_000;

interface AircraftEnvelope {
  aircraft?: unknown[];
  count?: number;
}

interface HudGlobeLoadingOverlayProps {
  /** Whether the flights layer is currently enabled in the dataManager. When
   *  false the overlay does not render — the operator opted out of the
   *  aircraft feed and we should not advertise it as "loading". */
  flightsLayerEnabled: boolean;
}

export function HudGlobeLoadingOverlay({
  flightsLayerEnabled,
}: HudGlobeLoadingOverlayProps) {
  const [aircraftCount, setAircraftCount] = useState<number | null>(null);
  const [elapsedMs, setElapsedMs] = useState(0);

  useEffect(() => {
    if (!flightsLayerEnabled) return;
    const start = Date.now();
    const tick = async () => {
      try {
        const env = await api<AircraftEnvelope>("/api/v1/globe/aircraft");
        const n =
          typeof env.count === "number"
            ? env.count
            : Array.isArray(env.aircraft)
              ? env.aircraft.length
              : 0;
        setAircraftCount(n);
      } catch (e) {
        // Swallow: a poll blip (401/5xx/timeout) must not blank the overlay.
        // The next tick retries; the TIMEOUT_MS ceiling guarantees we don't
        // sit here forever if the upstream is genuinely down.
        if (!(e instanceof ApiError) || e.status !== 401) {
          // Auth failures clear the key via api(); keep the overlay up.
        }
      }
      setElapsedMs(Date.now() - start);
    };
    void tick();
    const timer = setInterval(tick, POLL_MS);
    return () => clearInterval(timer);
  }, [flightsLayerEnabled]);

  if (!flightsLayerEnabled) return null;
  if (aircraftCount !== null && aircraftCount >= READY_THRESHOLD) return null;
  if (elapsedMs >= TIMEOUT_MS) return null;

  const timedOut = elapsedMs >= TIMEOUT_MS;
  // Show progress only after the first poll completes; before that the
  // count is "?" to avoid implying we already know the cache state.
  const countLabel =
    aircraftCount === null ? "…" : `${aircraftCount} 架`;
  const subtitle = timedOut
    ? "数据采集耗时较长 — 将在稍后后台继续刷新"
    : `正在扫描 8 个 ADS-B 区域 · ${countLabel} 已捕获`;

  return (
    <div
      className="hud-globe-loading-overlay"
      data-testid="hud-globe-loading-overlay"
      role="status"
      aria-live="polite"
    >
      <div className="hud-globe-loading-card">
        <div className="hud-globe-loading-spinner" aria-hidden="true" />
        <div className="hud-globe-loading-title">正在初始化全球视图</div>
        <div className="hud-globe-loading-subtitle">{subtitle}</div>
      </div>
    </div>
  );
}