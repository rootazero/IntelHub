// Kind display helper (§FE-RADAR-i18n). Single source of truth for
// rendering a kind value as a human-readable label across all surfaces
// (Radar dropdown + chips + detail drawer + MonitorMap legend + the
// events tooltip).
//
// Resolution order:
//   1. `enum.kind.<value>` from the active language dict.
//   2. English fallback (lookup the English dict; if missing, the
//      value is unknown in both dictionaries).
//   3. TitleCase fallback — turns `festival_holiday_past` into
//      `Festival Holiday Past`. Used for kind strings introduced by
//      new sources whose i18n entries haven't landed yet.
//
// This module is intentionally framework-free: it imports nothing
// React. The React surface uses it via `useHumanizeKind()` (see
// ./index.tsx) which subscribes to the active language.

import type { Dict } from "./en";
import { en } from "./en";
import { zh } from "./zh";

const DICTS: Record<"en" | "zh", Dict> = { en, zh };

/** Pure title-case transformation for snake_case / kebab-case kind ids.
 *  `festival_holiday_past` → `Festival Holiday Past`. */
export function titleCaseKind(value: string): string {
  if (!value) return "";
  return value
    .replace(/[_-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .replace(/\b\w/g, (c) => c.toUpperCase());
}

/** Look up a kind in a single dict; returns undefined if missing. */
function lookup(dict: Dict, value: string): string | undefined {
  const node = (dict.enum as Record<string, unknown> | undefined)?.kind as
    | Record<string, string>
    | undefined;
  return node?.[value];
}

/** Resolve a kind value to a human-readable label for the given lang.
 *  Never returns the empty string — falls through every layer of
 *  fallback so the caller never has to special-case "unknown kind". */
export function humanizeKind(value: string, lang: "en" | "zh" = "en"): string {
  if (!value) return "";
  const fromActive = lookup(DICTS[lang], value);
  if (fromActive) return fromActive;
  if (lang !== "en") {
    const fromEn = lookup(DICTS.en, value);
    if (fromEn) return fromEn;
  }
  // TitleCase fallback — applies to the original snake_case id.
  return titleCaseKind(value);
}

/** Snapshot of all known kinds: any kind the i18n dict has a label for.
 *  Used by Radar's dynamic dropdown to seed the option list before the
 *  first sweep lands (so users see the static taxonomy even on an
 *  empty map). Sorted alphabetically for stable display order. */
export function knownKinds(): string[] {
  const node = (en.enum as Record<string, unknown> | undefined)?.kind as
    | Record<string, string>
    | undefined;
  if (!node) return [];
  return Object.keys(node).sort();
}