// Kind display helper (§FE-RADAR-i18n + §FE-RADAR-PRIMARY-CATEGORIES).
// Single source of truth for rendering kind values as human-readable
// labels across all surfaces (Radar dropdown + chips + detail drawer +
// MonitorMap legend + events tooltip).
//
// Resolution order for humanizeKind (L2 — granular kinds):
//   1. `enum.kind.<value>` from the active language dict (the L2 dict
//      holds granular kinds like `festival_holiday_imminent_priority`,
//      `hdx_hapi_funding_priority`, etc.).
//   2. English fallback (lookup the English dict; if missing, the
//      value is unknown in both dictionaries).
//   3. TitleCase fallback — turns `festival_holiday_past` into
//      `Festival Holiday Past`. Used for kind strings introduced by new
//      sources whose i18n entries haven't landed yet.
//
// Resolution order for humanizeL1 (L1 — primary display kinds):
//   1. `enum.l1Kind.<value>` from the active language dict (the L1
//      dict holds the 15-category OSINT-standard taxonomy: conflict,
//      political, disaster, climate, health, cyber, maritime, aviation,
//      economic, news, humanitarian, holiday, research, wildlife, other).
//   2. English fallback.
//   3. TitleCase fallback.
//
// Why two helpers? L2 is machine-facing (Signal.kind on every event;
// agent/MCP queries; detail-drawer precise labels). L1 is human-facing
// (Radar chips + dropdown + legend aggregate 70+ L2 kinds into 15
// buckets so users see a concise taxonomy). Keeping the two dicts
// separate avoids key collisions (e.g. L1 `conflict` ≠ L2 `conflict`
// value, though they happen to look similar today).
//
// This module is intentionally framework-free: it imports nothing React.
// React surface uses humanizeKind() / humanizeL1() via useHumanizeKind()
// / useHumanizeL1() (see ./index.tsx) which subscribe to the active
// language.

import type { Dict } from "./en";
import { en } from "./en";
import { zh } from "./zh";

const DICTS: Record<"en" | "zh", Dict> = { en, zh };

type KindSection = "kind" | "l1Kind";

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

/** Look up a kind in a single dict under a given section; returns
 *  undefined if missing. The two sections (`kind` = L2 granular,
 *  `l1Kind` = L1 primary) carry disjoint namespaces. */
function lookup(dict: Dict, section: KindSection, value: string): string | undefined {
  const node = (dict.enum as Record<string, unknown> | undefined)?.[section] as
    | Record<string, string>
    | undefined;
  return node?.[value];
}

/** Look up a value in the active language, then fall back to English.
 *  Used by both humanizeKind and humanizeL1 to keep the two-tier
 *  fallback contract consistent. */
function lookupWithFallback(lang: "en" | "zh", section: KindSection, value: string): string | undefined {
  const fromActive = lookup(DICTS[lang], section, value);
  if (fromActive) return fromActive;
  if (lang !== "en") {
    return lookup(DICTS.en, section, value);
  }
  return undefined;
}

/** Resolve an L2 kind value to a human-readable label for the given
 *  lang. L2 = granular kinds emitted by monitor sources (Signal.kind
 *  on every geo_event). Never returns the empty string — falls through
 *  every layer of fallback so the caller never has to special-case
 *  "unknown kind". */
export function humanizeKind(value: string, lang: "en" | "zh" = "en"): string {
  if (!value) return "";
  const fromDict = lookupWithFallback(lang, "kind", value);
  if (fromDict) return fromDict;
  return titleCaseKind(value);
}

/** Resolve an L1 (primary display) kind value to a human-readable
 *  label. L1 = the 15-category OSINT-standard taxonomy that Radar
 *  chips + dropdown + legend aggregate to. Falls back to TitleCase if
 *  the value is not in the L1 dict. */
export function humanizeL1(value: string, lang: "en" | "zh" = "en"): string {
  if (!value) return "";
  const fromDict = lookupWithFallback(lang, "l1Kind", value);
  if (fromDict) return fromDict;
  return titleCaseKind(value);
}

/** Snapshot of all L2 kinds the i18n dict has a label for. Used by
 *  Radar's dropdown as the seed option list before the first sweep
 *  lands (so users see the static taxonomy even on an empty map).
 *  Sorted alphabetically for stable display order. */
export function knownKinds(): string[] {
  const node = (en.enum as Record<string, unknown> | undefined)?.kind as
    | Record<string, string>
    | undefined;
  if (!node) return [];
  return Object.keys(node).sort();
}

/** Snapshot of all L1 (primary) kinds the i18n dict has a label for.
 *  Always 15 entries by construction (matches PRIMARY_KINDS in
 *  kindmeta). Sorted alphabetically for stable display order. */
export function knownPrimaryKinds(): string[] {
  const node = (en.enum as Record<string, unknown> | undefined)?.l1Kind as
    | Record<string, string>
    | undefined;
  if (!node) return [];
  return Object.keys(node).sort();
}