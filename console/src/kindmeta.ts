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

export const kindColor = (k: string): string =>
  KIND_COLORS[k] ?? (PRIMARY_COLOR as Record<string, string>)[k] ?? KIND_COLORS.other;

// Severity → size channel (flash biggest, info smallest/dimmest).
export const sevRadius = (s: string): number =>
  s === "flash" ? 7 : s === "priority" ? 6 : s === "routine" ? 4 : 3;

// ---------------------------------------------------------------------------
// §FE-RADAR-PRIMARY-CATEGORIES (2026-09-28) — L1 (primary display) taxonomy.
//
// Two-level design:
//   L2 = the granular kind stored in `Signal.kind` / `geo_events.kind`.
//        ~70+ values, machine-readable, agent/MCP/detail-drawer facing.
//        Unchanged by this commit.
//   L1 = a 15-category OSINT-standard taxonomy for human viewing.
//        Radar chips + dropdown + FilterBar + MonitorMap legend all use L1.
//
// Each L2 routes to exactly one L1 via KIND_TO_PRIMARY below. New sources
// that emit an unknown L2 fall through to `other` so the chip strip never
// silently grows beyond 15 slots. PRIMARY_KINDS is the canonical ordering
// (also drives display order in Radar chips + dropdown + legend).
//
// L1 colors are picked to be distinct from each other on the dark basemap
// while staying consistent with the existing L2 palette where the L1 name
// already had a color (e.g. L1 `conflict` keeps `#ff3355` from L2).
// ---------------------------------------------------------------------------

export const PRIMARY_KINDS = [
  "conflict",
  "political",
  "disaster",
  "climate",
  "health",
  "cyber",
  "maritime",
  "aviation",
  "economic",
  "news",
  "humanitarian",
  "holiday",
  "research",
  "wildlife",
  "other",
] as const;

export type PrimaryKind = (typeof PRIMARY_KINDS)[number];

/** L2 → L1 routing table. Every L2 kind emitted by a monitor source must
 *  appear here; missing entries fall through to `other` via primaryOf().
 *  Ordered to mirror PRIMARY_KINDS grouping (so additions are easy to spot
 *  in diffs). */
export const KIND_TO_PRIMARY: Record<string, PrimaryKind> = {
  // conflict
  conflict: "conflict",
  military: "conflict",
  // political
  political: "political",
  politician_trade_house_large: "political",
  politician_trade_senate: "political",
  politician_trade_info: "political",
  // disaster (quake + fire + flood)
  disaster: "disaster",
  quake: "disaster",
  fire: "disaster",
  fire_extreme: "disaster",
  fire_high: "disaster",
  fire_significant: "disaster",
  fire_top50: "disaster",
  flood: "disaster",
  flood_minor: "disaster",
  flood_moderate: "disaster",
  flood_major: "disaster",
  flood_action: "disaster",
  // climate (env / atmosphere)
  climate: "climate",
  air_quality: "climate",
  weather_extreme: "climate",
  // health
  health: "health",
  disease_outbreak_priority: "health",
  disease_outbreak_routine: "health",
  disease_outbreak_info: "health",
  // cyber (incl. sanction / threat cluster / scanner / secret-leak / compliance)
  cyber: "cyber",
  sanction: "cyber",
  threatcluster_threat_critical: "cyber",
  threatcluster_threat_high: "cyber",
  threatcluster_threat_low: "cyber",
  threatcluster_vuln_active: "cyber",
  threatcluster_vuln_background: "cyber",
  threatcluster_vuln_critical: "cyber",
  scanner_malicious: "cyber",
  scanner_benign: "cyber",
  scanner_unknown: "cyber",
  secret_leak_broad: "cyber",
  secret_leak_triggered: "cyber",
  secret_leak_ignored: "cyber",
  secret_leak_info: "cyber",
  compliance_match_exact_priority: "cyber",
  compliance_match_partial_routine: "cyber",
  compliance_match_low_confidence: "cyber",
  // maritime
  maritime: "maritime",
  chokepoint_info: "maritime",
  chokepoint_bulker_routine: "maritime",
  chokepoint_tanker_priority: "maritime",
  chokepoint_tanker_routine: "maritime",
  vessel_sanctions_red: "maritime",
  vessel_sanctions_yellow: "maritime",
  vessel_verdict_info: "maritime",
  vessel_ownership_opaque: "maritime",
  vessel_vetting_d_e: "maritime",
  // aviation
  flight: "aviation",
  // economic (financial rolled in)
  economic: "economic",
  financial: "economic",
  // news (consolidates the 7 Helium + RSS sub-severities)
  news: "news",
  news_priority: "news",
  news_routine: "news",
  news_info: "news",
  news_headline: "news",
  news_significant: "news",
  news_peripheral: "news",
  // humanitarian (HDX HAPI + hdx_humanitarian CKAN catalog)
  hdx_hapi_national_risk_priority: "humanitarian",
  hdx_hapi_national_risk_routine: "humanitarian",
  hdx_hapi_national_risk_info: "humanitarian",
  hdx_hapi_funding_priority: "humanitarian",
  hdx_hapi_funding_routine: "humanitarian",
  hdx_hapi_funding_info: "humanitarian",
  hdx_humanitarian: "humanitarian",
  // holiday (consolidates nager_date + festival_holidays)
  holiday: "holiday",
  festival_holiday_past: "holiday",
  festival_holiday_imminent_priority: "holiday",
  festival_holiday_imminent_routine: "holiday",
  festival_holiday_upcoming: "holiday",
  // research (semantic_scholar papers)
  paper_priority: "research",
  paper_routine: "research",
  paper_info: "research",
  // wildlife (GBIF)
  wildlife_priority: "wildlife",
  wildlife_routine: "wildlife",
  wildlife_info: "wildlife",
  // other (catch-all bucket for radiation + daylight + future unknowns)
  radiation: "other",
  daylight: "other",
  other: "other",
};

/** L1 chip / legend colors. Reuses L2 hues where the L1 name already
 *  had one in KIND_COLORS; introduces fresh hues for new buckets. */
export const PRIMARY_COLOR: Record<PrimaryKind, string> = {
  conflict: "#ff3355",
  political: "#b388ff",
  disaster: "#ff7a1a",
  climate: "#2dd4bf",
  health: "#f72585",
  cyber: "#00f5d4",
  maritime: "#3a6ff7",
  aviation: "#7cc7ff",
  economic: "#34d399",
  news: "#c8cdd4",
  humanitarian: "#80ed99",
  holiday: "#facc15",
  research: "#a78bfa",
  wildlife: "#86efac",
  other: "#8a8f98",
};

/** Map an L2 kind to its L1 primary category. Unknown / empty L2 inputs
 *  fall through to `other` so the chip strip is bounded. */
export const primaryOf = (kind: string): PrimaryKind =>
  KIND_TO_PRIMARY[kind] ?? "other";