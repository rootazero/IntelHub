// GEV P9 regional briefing adapter — fetches the hub's `GET /api/v1/gev/weather`
// and `GET /api/v1/gev/summary` endpoints (T2) and normalizes them into the
// React HUD's briefing view model, plus owns the summary-bullet auto-rotation.
//
// The vendored cockpitBriefing.js exports `this`-bound mixin methods
// (showBriefPage / startBriefRotation / renderRegionalBrief …) that walk DOM
// refs on the cockpitController — we do NOT instantiate that controller (D1).
// This adapter keeps the vendor's cadence + formatting constants
// (COCKPIT_BRIEF_ROTATE_MS, formatCockpitWindDirection, COCKPIT_BRIEF_PAGES)
// and re-implements the small rotation/state machine in pure TS.
//
// Empty-bullet handling (T2 finding): acled/reliefweb/gdelt do NOT exist in
// hub-core yet, so `gev_summary` returns 200 with `bullets: []` +
// `sources: ["cache"]` (spec §3.3 空载防御). That is an EXPECTED "no data"
// state — surfaced as `summary.empty === true`, never thrown, never retried.
import {
  COCKPIT_BRIEF_PAGES,
  COCKPIT_BRIEF_ROTATE_MS,
  formatCockpitWindDirection,
} from "gev-engine/src/ui/cockpitPresentation.js";
import type { ApiFetch } from "../../gev-adapters/http";

export type WeatherMetricKey =
  | "temperature"
  | "wind"
  | "precipitation"
  | "visibility";

export interface WeatherMetric {
  key: WeatherMetricKey;
  label: string;
  /** Formatted display value, e.g. "12°C" / "25 KTS" / "0.4 MM" / "10 KM". */
  value: string | null;
  /** Secondary detail, e.g. wind direction "N · 250°". */
  detail: string | null;
}

export interface SummaryBullet {
  text: string;
  sourceUrl: string | null;
  ageHours: number | null;
}

export interface BriefPageMeta {
  id: string;
  kicker: string;
  subtitle: string;
  source: string;
}

export interface Briefing {
  weather: {
    source: string;
    fetchedAt: string | null;
    metrics: WeatherMetric[];
    /** True when the weather endpoint errored (503) — grey banner, no retry. */
    degraded: boolean;
  };
  summary: {
    entityId: string;
    sources: string[];
    bullets: SummaryBullet[];
    /** True when bullets is empty (hub stub — no upstream aggregators yet). */
    empty: boolean;
    /** True when the summary endpoint itself failed (non-ok transport). */
    degraded: boolean;
  };
}

export interface BriefingHandle {
  fetch(
    lat: number,
    lon: number,
    entityId: string,
    opts?: { signal?: AbortSignal },
  ): Promise<Briefing>;
  /** Begin auto-rotation over summary bullets at COCKPIT_BRIEF_ROTATE_MS. */
  start(briefing?: Briefing): void;
  stop(): void;
  next(): number;
  prev(): number;
  index(): number;
  total(): number;
  /** The vendor's three brief pages (signals/news/local) as metadata. */
  pages(): readonly BriefPageMeta[];
  current(): Briefing | null;
  destroy(): void;
}

const EMPTY_WEATHER: Briefing["weather"] = {
  source: "unknown",
  fetchedAt: null,
  metrics: [],
  degraded: true,
};

const EMPTY_SUMMARY: Briefing["summary"] = {
  entityId: "",
  sources: [],
  bullets: [],
  empty: true,
  degraded: true,
};

function isFiniteNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function normalizeWeather(body: any): Briefing["weather"] {
  const metrics: WeatherMetric[] = [];
  const push = (key: WeatherMetricKey, label: string, value: string | null, detail: string | null = null) =>
    metrics.push({ key, label, value, detail });

  push(
    "temperature",
    "TEMP",
    isFiniteNumber(body?.temperature_c)
      ? `${Math.round(body.temperature_c)}°C`
      : null,
  );
  push(
    "wind",
    "WIND",
    isFiniteNumber(body?.wind_speed_kts)
      ? `${Math.round(body.wind_speed_kts)} KTS`
      : null,
    isFiniteNumber(body?.wind_direction_deg)
      ? formatCockpitWindDirection(body.wind_direction_deg)
      : null,
  );
  push(
    "precipitation",
    "PRECIP",
    isFiniteNumber(body?.precipitation_mm)
      ? `${body.precipitation_mm.toFixed(1)} MM`
      : null,
  );
  push(
    "visibility",
    "VIS",
    isFiniteNumber(body?.visibility_m)
      ? `${Math.round(body.visibility_m / 1000)} KM`
      : null,
  );

  return {
    source: typeof body?.source === "string" ? body.source : "unknown",
    fetchedAt: typeof body?.fetched_at === "string" ? body.fetched_at : null,
    metrics,
    degraded: false,
  };
}

function normalizeSummary(body: any, entityId: string): Briefing["summary"] {
  const raw = Array.isArray(body?.bullets) ? body.bullets : [];
  const bullets: SummaryBullet[] = raw.map((b: any) => ({
    text: typeof b?.text === "string" ? b.text : String(b?.text ?? ""),
    sourceUrl: typeof b?.source_url === "string" ? b.source_url : null,
    ageHours: isFiniteNumber(b?.age_hours) ? (b.age_hours as number) : null,
  }));

  return {
    entityId: typeof body?.entity_id === "string" ? body.entity_id : entityId,
    sources: Array.isArray(body?.sources) ? body.sources : [],
    bullets,
    empty: bullets.length === 0,
    degraded: false,
  };
}

function isAbortError(error: unknown): boolean {
  return (
    (error instanceof DOMException && error.name === "AbortError") ||
    (error instanceof Error && error.name === "AbortError")
  );
}

export function mountCockpitBriefing(apiFetch: ApiFetch): BriefingHandle {
  if (typeof apiFetch !== "function") {
    throw new TypeError("mountCockpitBriefing: apiFetch must be a function");
  }

  let current: Briefing | null = null;
  let bulletIndex = 0;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let destroyed = false;

  const total = () => current?.summary.bullets.length ?? 0;

  function next(): number {
    const count = total();
    if (count === 0) return 0;
    bulletIndex = (bulletIndex + 1) % count;
    return bulletIndex;
  }

  function prev(): number {
    const count = total();
    if (count === 0) return 0;
    bulletIndex = (bulletIndex - 1 + count) % count;
    return bulletIndex;
  }

  function stop() {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
  }

  function start(briefing?: Briefing): void {
    if (briefing) current = briefing;
    stop();
    if (destroyed) return;
    if (total() === 0) return;
    const tick = () => {
      next();
      if (destroyed) return;
      timer = setTimeout(tick, COCKPIT_BRIEF_ROTATE_MS);
    };
    timer = setTimeout(tick, COCKPIT_BRIEF_ROTATE_MS);
  }

  return {
    async fetch(lat, lon, entityId, opts = {}) {
      if (destroyed) return current ?? { weather: EMPTY_WEATHER, summary: { ...EMPTY_SUMMARY } };

      // Weather first (D3: a 503 degrades the briefing, never rejects the
      // caller into a retry storm).
      let weather: Briefing["weather"] = EMPTY_WEATHER;
      try {
        const wres = await apiFetch(
          `/api/v1/gev/weather?lat=${encodeURIComponent(lat)}&lon=${encodeURIComponent(lon)}`,
          { signal: opts.signal },
        );
        if (wres.ok) weather = normalizeWeather(await wres.json());
      } catch (error) {
        if (isAbortError(error)) throw error;
        weather = { ...EMPTY_WEATHER };
      }

      let summary: Briefing["summary"] = {
        ...EMPTY_SUMMARY,
        entityId,
      };
      try {
        const sres = await apiFetch(
          `/api/v1/gev/summary?entity_id=${encodeURIComponent(entityId)}`,
          { signal: opts.signal },
        );
        if (sres.ok) summary = normalizeSummary(await sres.json(), entityId);
      } catch (error) {
        if (isAbortError(error)) throw error;
        summary = { ...EMPTY_SUMMARY, entityId };
      }

      bulletIndex = 0;
      current = { weather, summary };
      return current;
    },
    start,
    stop,
    next,
    prev,
    index: () => bulletIndex,
    total,
    pages: () =>
      (COCKPIT_BRIEF_PAGES as unknown as BriefPageMeta[]).map((p) => ({
        id: p.id,
        kicker: p.kicker,
        subtitle: p.subtitle,
        source: p.source,
      })),
    current: () => current,
    destroy() {
      destroyed = true;
      stop();
      current = null;
      bulletIndex = 0;
    },
  };
}
