// Satellite layer — TLE catalog from hub PG (localStorage 1h cache),
// SGP4 propagation in-browser (positions never touch PG, spec §1.2).

import * as satellite from "satellite.js";
import { api } from "../api";

export interface SatTle {
  norad_id: number;
  name: string;
  category: string;
  tle1: string;
  tle2: string;
  epoch: string;
}

export interface SatRec {
  tle: SatTle;
  satrec: satellite.SatRec;
}

export interface SatPosition {
  norad_id: number;
  name: string;
  category: string;
  lat: number;
  lon: number;
  altKm: number;
}

const CACHE_KEY = "intelhub.globe.tles";
const CACHE_MS = 3_600_000;

export async function loadSatellites(): Promise<SatTle[]> {
  try {
    const raw = localStorage.getItem(CACHE_KEY);
    if (raw) {
      const c = JSON.parse(raw) as { ts: number; items: SatTle[] };
      if (Date.now() - c.ts < CACHE_MS) return c.items;
    }
  } catch {
    /* corrupt cache → refetch */
  }
  const r = await api<{ count: number; items: SatTle[] }>("/api/v1/globe/satellites");
  try {
    localStorage.setItem(CACHE_KEY, JSON.stringify({ ts: Date.now(), items: r.items }));
  } catch {
    /* quota — non-fatal */
  }
  return r.items;
}

export function buildSatRecs(tles: SatTle[]): SatRec[] {
  const out: SatRec[] = [];
  for (const tle of tles) {
    try {
      out.push({ tle, satrec: satellite.twoline2satrec(tle.tle1, tle.tle2) });
    } catch {
      /* bad TLE → skip */
    }
  }
  return out;
}

export function propagateAll(recs: SatRec[], date: Date): SatPosition[] {
  const gmst = satellite.gstime(date);
  const out: SatPosition[] = [];
  for (const r of recs) {
    const pv = satellite.propagate(r.satrec, date);
    const pos = pv?.position;
    if (!pos) continue;
    const geo = satellite.eciToGeodetic(pos as satellite.EciVec3<number>, gmst);
    out.push({
      norad_id: r.tle.norad_id,
      name: r.tle.name,
      category: r.tle.category,
      lat: satellite.degreesLat(geo.latitude),
      lon: satellite.degreesLong(geo.longitude),
      altKm: geo.height,
    });
  }
  return out;
}
