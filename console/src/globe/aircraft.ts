// Aircraft live layer — polls hub Redis snapshot (never upstream direct,
// spec §1.3). Dead-reckoning extrapolates positions between 15s polls.

import { useEffect, useState } from "react";
import { api } from "../api";

export interface AircraftPoint {
  hex: string;
  flight: string | null;
  lat: number;
  lon: number;
  alt_m: number;
  gs: number | null;
  track: number | null;
  squawk: string | null;
  mil: boolean;
  age_s?: number;
}

// Envelope (spec §2.4, Task 5 ruling): {ts,count,coverage,cycle_secs,
// last_tick,aircraft[]}. Degraded body is {stale:true,aircraft:[]} — stale
// is the only reliable discriminator; every other field must be tolerated
// as missing.
export interface AircraftSnapshot {
  ts?: string;
  count?: number;
  coverage?: string;
  cycle_secs?: number;
  last_tick?: string;
  stale?: boolean;
  aircraft: AircraftPoint[];
}

export function useAircraft(
  pollMs = 15000,
): { snap: AircraftSnapshot | null; fetchedAt: number } {
  const [snap, setSnap] = useState<AircraftSnapshot | null>(null);
  const [fetchedAt, setFetchedAt] = useState(0);
  useEffect(() => {
    let dead = false;
    const tick = async () => {
      try {
        const s = await api<AircraftSnapshot>("/api/v1/globe/aircraft");
        if (!dead) {
          // Tolerate the degraded {stale:true} body — aircraft may be absent.
          if (!Array.isArray(s.aircraft)) s.aircraft = [];
          setSnap(s);
          setFetchedAt(Date.now());
        }
      } catch {
        /* keep last good snapshot; stale flag handles death */
      }
    };
    void tick();
    const t = setInterval(tick, pollMs);
    return () => {
      dead = true;
      clearInterval(t);
    };
  }, [pollMs]);
  return { snap, fetchedAt };
}

/**
 * Great-circle dead reckoning from gs (knots) + track (deg).
 * dtSec is capped at MAX_DR_SEC (worst-case snapshot age = cycle_secs +
 * margin) so long-idle tabs never extrapolate a track off the globe.
 */
export function deadReckon(ac: AircraftPoint, dtSec: number): [number, number] {
  const dt = Math.min(Math.max(dtSec, 0), MAX_DR_SEC);
  if (!ac.gs || ac.track == null || dt <= 0) return [ac.lat, ac.lon];
  const d = ac.gs * 0.514444 * dt; // meters
  const R = 6_371_000;
  const brg = (ac.track * Math.PI) / 180;
  const lat1 = (ac.lat * Math.PI) / 180;
  const lon1 = (ac.lon * Math.PI) / 180;
  const lat2 = Math.asin(Math.sin(lat1) * Math.cos(d / R) + Math.cos(lat1) * Math.sin(d / R) * Math.cos(brg));
  const lon2 = lon1 + Math.atan2(
    Math.sin(brg) * Math.sin(d / R) * Math.cos(lat1),
    Math.cos(d / R) - Math.sin(lat1) * Math.sin(lat2),
  );
  return [(lat2 * 180) / Math.PI, (lon2 * 180) / Math.PI];
}

/** Max dead-reckoning horizon: 210s cycle + margin (controller ruling). */
export const MAX_DR_SEC = 240;
