// GEV P17 cockpit HUD element visibility — per-element show/hide map with
// localStorage persistence. Mirrors vision-mount.ts shape: types + storage
// helpers + safe parse fallback. Default = all visible so first load looks
// identical to pre-P17 (no breakage for existing users).

export const COCKPIT_ELEMENT_KEYS = [
  "compass",
  "altimeter",
  "speedRuler",
  "altitudeLadder",
  "speedTape",
  "pitchLadder",
  "bankIndicator",
  "vsiChevron",
] as const;

export type CockpitElementKey = (typeof COCKPIT_ELEMENT_KEYS)[number];

export type ElementVisibility = Record<CockpitElementKey, boolean>;

/** localStorage key holding the user's per-element visibility pick. Exported
 *  so every writer/reader (HudCockpitElementSwitch, cockpit-store rehydrate)
 *  shares one literal instead of re-typing it. */
export const ELEMENT_VISIBILITY_STORAGE_KEY =
  "intelhub.cockpit.elementVisibility";

export const DEFAULT_ELEMENT_VISIBILITY: ElementVisibility = {
  compass: true,
  altimeter: true,
  speedRuler: true,
  altitudeLadder: true,
  speedTape: true,
  pitchLadder: true,
  bankIndicator: true,
  vsiChevron: true,
};

export function isCockpitElementKey(value: unknown): value is CockpitElementKey {
  return (
    typeof value === "string" &&
    (COCKPIT_ELEMENT_KEYS as readonly string[]).includes(value)
  );
}

/** Rehydrate the visibility map from localStorage. On any failure (missing
 *  key, invalid JSON, unknown element key), fall back to
 *  DEFAULT_ELEMENT_VISIBILITY so the cockpit renders fully — this matches
 *  pre-P17 behavior, so the change is invisible to users on first load. */
export function readPersistedElementVisibility(): ElementVisibility {
  let raw: string | null = null;
  try {
    raw = localStorage.getItem(ELEMENT_VISIBILITY_STORAGE_KEY);
  } catch {
    return { ...DEFAULT_ELEMENT_VISIBILITY };
  }
  if (!raw) return { ...DEFAULT_ELEMENT_VISIBILITY };
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return { ...DEFAULT_ELEMENT_VISIBILITY };
  }
  if (!parsed || typeof parsed !== "object") {
    return { ...DEFAULT_ELEMENT_VISIBILITY };
  }
  const result: ElementVisibility = { ...DEFAULT_ELEMENT_VISIBILITY };
  for (const [key, value] of Object.entries(parsed)) {
    if (isCockpitElementKey(key) && typeof value === "boolean") {
      result[key] = value;
    }
  }
  return result;
}

/** Best-effort persist. Quota errors / private-mode blocks are swallowed —
 *  same convention as vision-mount's mode setter. */
export function persistElementVisibility(visibility: ElementVisibility): void {
  try {
    localStorage.setItem(
      ELEMENT_VISIBILITY_STORAGE_KEY,
      JSON.stringify(visibility),
    );
  } catch {
    /* private mode: persistence is best-effort */
  }
}