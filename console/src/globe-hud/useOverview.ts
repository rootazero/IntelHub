// Overview poll for the GEV globe HUD bars (T11).
//
// The globe page owns ONE poll and hands the snapshot to both bars (top: open
// alerts; bottom: collector health + 24 h event count) — two independent
// intervals would double the /api/v1/overview load and let the two strips show
// data from different instants.
//
// Transport reuses the console client (`src/api`), matching the existing page
// pattern (Overview.tsx / Monitor.tsx: fetch on mount + setInterval, swallow
// poll errors so a single blip never blanks the UI).
import { useEffect, useRef, useState } from "react";
import { api } from "../api";

/** Poll cadence (task-11 brief). */
export const OVERVIEW_POLL_MS = 15_000;

/** Redis health cell for one monitor collector (`hub:monitor:health`), as
 *  surfaced by GET /api/v1/overview under radar.monitor.sources. */
export interface GlobeSourceHealth {
  name: string;
  /** "ok" | "error" | "absent" | "unknown" | … (collector-written). */
  state?: string;
  /** New rows produced by the latest sweep. */
  last_new?: number;
  ts?: string | null;
  detail?: string | null;
}

/** Narrow slice of GET /api/v1/overview that the HUD bars consume. All fields
 *  optional: the endpoint is versioned independently and the bars must stay
 *  renderable against an older hub. */
export interface OverviewData {
  alerts?: { open?: number };
  radar?: {
    geo_events_24h?: number;
    monitor?: {
      up?: boolean;
      sources_ok?: number;
      sources_total?: number;
      sources?: GlobeSourceHealth[];
    };
  };
}

export interface OverviewState {
  overview: OverviewData | null;
  /** True when the most recent poll failed (the previous snapshot, if any, is
   *  kept — a poll blip must not blank the bars). */
  error: boolean;
}

export function useOverview(intervalMs = OVERVIEW_POLL_MS): OverviewState {
  const [state, setState] = useState<OverviewState>({
    overview: null,
    error: false,
  });
  // Guards the state write after unmount (the poll is async, the page may be
  // left while a request is in flight).
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    const load = () => {
      api<OverviewData>("/api/v1/overview").then(
        (overview) => {
          if (mounted.current) setState({ overview, error: false });
        },
        () => {
          if (mounted.current)
            setState((previous) => ({ ...previous, error: true }));
        },
      );
    };
    load();
    const timer = setInterval(load, intervalMs);
    return () => {
      mounted.current = false;
      clearInterval(timer);
    };
  }, [intervalMs]);

  return state;
}
