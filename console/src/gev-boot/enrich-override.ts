// GEV P13 T2 — raise the vendor's ambient enrichment budget via its QA seam.
//
// Why this module exists: the vendored flights layer fills ambient (untracked)
// aircraft enrichment from a token bucket whose defaults are deliberately
// conservative for politeness — ENRICH_AMBIENT_BUDGET_CEIL=300,
// ENRICH_AMBIENT_REFILL_TOKENS=150 per ENRICH_AMBIENT_REFILL_WINDOW_MS=300000
// (gev-engine/src/layers/flights/policy.js:259-271). With ~374 planes visible
// on the P13 default view, ~74 of them can never be classified within a single
// refresh window, so they keep the "yellow, no details" look.
//
// The vendor deliberately exposes `window.__GEV_ENRICH_AMBIENT_QA`
// (enrichment.js::_ambientBudgetKnobs, enrichment.js:132-141) as an approved
// runtime override — it is read LAZILY on every refill, so a pre-boot write
// applies from the first sweep, and a mid-run windowMs swap also works. This is
// the sanctioned escape hatch: it changes a knob, not the vendor source.
//
// We raise the ceiling so the whole visible set can be enriched in one window
// while keeping the refill cadence identical (400 / 5 min instead of 150 / 5
// min is still ~1.3 req/s worst case — a fraction of the 5/s drip the vendor
// documents as its real politeness bound on adsbdb).
//
// The values below are the spec (plan §Task 2) and are pinned by
// __tests__/enrich-override.test.ts.

export const ENRICH_OVERRIDE_DEFAULTS = {
  ceil: 800,
  refillTokens: 400,
  windowMs: 300_000,
} as const;

/** Caller-supplied partial override of ENRICH_OVERRIDE_DEFAULTS. */
export interface EnrichAmbientOverride {
  ceil?: number;
  refillTokens?: number;
  windowMs?: number;
}

type EnrichQaWindow = {
  __GEV_ENRICH_AMBIENT_QA?: Record<string, number>;
};

/**
 * Install the raised ambient budget on the QA seam. Idempotent: a caller (or a
 * headless QA harness like gev-engine/scripts/qa-enrich-ambient.mjs) that has
 * already published a complete 3-knob override wins — this function never
 * clobbers it. A partial/absent override is replaced by defaults + overrides.
 *
 * Returns nothing; the vendor reads `window.__GEV_ENRICH_AMBIENT_QA` lazily, so
 * there is no handle to hand back.
 */
export function applyEnrichAmbientOverride(
  overrides: EnrichAmbientOverride = {},
): void {
  const w = globalThis as unknown as EnrichQaWindow;
  const existing = w.__GEV_ENRICH_AMBIENT_QA;
  if (existing && Object.keys(existing).length >= 3) return;
  w.__GEV_ENRICH_AMBIENT_QA = { ...ENRICH_OVERRIDE_DEFAULTS, ...overrides };
}
