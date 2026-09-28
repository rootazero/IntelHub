// Shared Radar kind taxonomy (SP8): one palette for every map + chip.
// Kind is the PRIMARY color channel on dark basemaps; severity moved to
// size/pulse so a red dot unambiguously means "conflict", not "urgent".
//
// §FE-RADAR-i18n (2026-09-27): extended with 23 dynamic kinds emitted
// by monitor sources whose payloads surface as `kind` in geo_events.
// Previously these had no display name + no i18n entry, so Radar's
// bottom chips showed raw snake_case. Colors mirror the parent class
// (e.g. `fire_high` and `fire_extreme` reuse `fire`'s orange) so the
// palette stays small enough to read at a glance.
//
// New sources are expected to (a) append to KINDS below, (b) add an
// enum.kind.* entry in en.ts + zh.ts, and (c) reuse an existing color
// or pick a fresh one and add it to KIND_COLORS. Anything missing
// from this list will still display — humanizeKind() falls back to
// TitleCase for unknown values, but the color falls back to KIND_COLORS.other.

export const KINDS = [
  // Core taxonomy (original 17) — colors pre-defined.
  "conflict", "political", "military", "fire", "quake", "disaster",
  "flight", "maritime", "news", "financial", "health", "radiation",
  "cyber", "sanction", "economic", "climate", "other",

  // §FE-RADAR-i18n additions (2026-09-27): dynamic kinds emitted by
  // specific monitor sources. Each maps to a parent class color.
  // Fire sub-severity (Brazil INPE queimadas): same orange as `fire`.
  "fire_high", "fire_extreme",
  // News sub-severity (Helium RSS classifier): grey like `news`.
  "news_headline", "news_peripheral", "news_significant",
  // Open-Meteo extreme-weather alerts.
  "weather_extreme", "air_quality", "flood",
  // Sunrise/sunset daylight markers.
  "daylight",
  // Holidays — `holiday` from legacy nager_date; the four
  // festival_holiday_* are caldays sub-severities from Phase 4.4.
  "holiday",
  "festival_holiday_past",
  "festival_holiday_imminent_priority",
  "festival_holiday_imminent_routine",
  "festival_holiday_upcoming",
  // AIS / chokepoint (strait_of_hormuz, arcnautical): blue family.
  "chokepoint_info", "chokepoint_bulker_routine",
  "vessel_sanctions_red", "vessel_sanctions_yellow",
  "vessel_verdict_info", "vessel_ownership_opaque",
  // GreyNoise scanner reputation.
  "scanner_malicious", "scanner_benign", "scanner_unknown",
];

export const KIND_COLORS: Record<string, string> = {
  // Core 17 — distinct hues.
  conflict: "#ff3355",
  political: "#b388ff",
  military: "#a3b18a",
  fire: "#ff7a1a",
  quake: "#ffd60a",
  disaster: "#2f80ed",
  flight: "#7cc7ff",
  maritime: "#3a6ff7",
  news: "#c8cdd4",
  financial: "#34d399",
  health: "#f72585",
  radiation: "#ccff00",
  cyber: "#00f5d4",
  sanction: "#ff9e00",
  economic: "#80ed99",
  climate: "#2dd4bf",
  other: "#8a8f98",

  // §FE-RADAR-i18n additions — reuse parent hues; only one fresh slot.
  // Fire sub-severity: same orange as `fire`.
  fire_high: "#ff7a1a",
  fire_extreme: "#ff4500", // a touch redder than `fire` so extreme pops on the map
  // News sub-severity: same grey as `news`.
  news_headline: "#c8cdd4",
  news_peripheral: "#c8cdd4",
  news_significant: "#c8cdd4",
  // Weather / daylight: cyan/teal family.
  weather_extreme: "#22d3ee",
  air_quality: "#06b6d4",
  flood: "#0284c7",
  daylight: "#fbbf24",
  // Holidays: warm yellow family so they read as "calendar".
  holiday: "#facc15",
  festival_holiday_past: "#a16207",
  festival_holiday_imminent_priority: "#facc15",
  festival_holiday_imminent_routine: "#fde68a",
  festival_holiday_upcoming: "#fef3c7",
  // AIS / chokepoint: blue family (maritime lineage).
  chokepoint_info: "#60a5fa",
  chokepoint_bulker_routine: "#3b82f6",
  vessel_sanctions_red: "#dc2626",
  vessel_sanctions_yellow: "#fbbf24",
  vessel_verdict_info: "#3a6ff7",
  vessel_ownership_opaque: "#6366f1",
  // GreyNoise: green/red/grey for benign/malicious/unknown.
  scanner_malicious: "#dc2626",
  scanner_benign: "#34d399",
  scanner_unknown: "#8a8f98",
};

export const kindColor = (k: string): string => KIND_COLORS[k] ?? KIND_COLORS.other;

// Severity → size channel (flash biggest, info smallest/dimmest).
export const sevRadius = (s: string): number =>
  s === "flash" ? 7 : s === "priority" ? 6 : s === "routine" ? 4 : 3;