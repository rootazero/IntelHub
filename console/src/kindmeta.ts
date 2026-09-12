// Shared Radar kind taxonomy (SP8): one palette for every map + chip.
// Kind is the PRIMARY color channel on dark basemaps; severity moved to
// size/pulse so a red dot unambiguously means "conflict", not "urgent".
export const KINDS = [
  "conflict", "political", "military", "fire", "quake", "disaster",
  "flight", "maritime", "news", "financial", "health", "radiation",
  "cyber", "sanction", "economic", "climate", "other",
];

export const KIND_COLORS: Record<string, string> = {
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
};

export const kindColor = (k: string): string => KIND_COLORS[k] ?? KIND_COLORS.other;

// Severity → size channel (flash biggest, info smallest/dimmest).
export const sevRadius = (s: string): number =>
  s === "flash" ? 7 : s === "priority" ? 6 : s === "routine" ? 4 : 3;
