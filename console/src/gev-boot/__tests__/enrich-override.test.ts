// GEV P13 T2 — vendor ambient enrichment budget override.
//
// The failure this pins: the vendor ambient enrichment bucket defaults to
// ENRICH_AMBIENT_BUDGET_CEIL=300 (gev-engine/src/layers/flights/policy.js:259),
// so roughly a quarter of the ~374 visible aircraft never get classified within
// a refresh window and stay visually "yellow with no details". The vendor's
// approved escape hatch is the QA seam window.__GEV_ENRICH_AMBIENT_QA, read
// lazily by enrichment.js::_ambientBudgetKnobs (enrichment.js:132-141). This
// module ships the production-shaped override.
import { beforeEach, describe, expect, it } from "vitest";
import {
  ENRICH_OVERRIDE_DEFAULTS,
  applyEnrichAmbientOverride,
} from "../enrich-override";

describe("applyEnrichAmbientOverride", () => {
  beforeEach(() => {
    delete (globalThis as any).__GEV_ENRICH_AMBIENT_QA;
  });

  it("sets window.__GEV_ENRICH_AMBIENT_QA with raised budget", () => {
    // The constants are the spec (plan §Task 2) — pin them so a refactor
    // cannot silently drop back under the vendor's 300-token ceiling.
    expect(ENRICH_OVERRIDE_DEFAULTS).toEqual({
      ceil: 800,
      refillTokens: 400,
      windowMs: 300_000,
    });
    applyEnrichAmbientOverride();
    const qa = (globalThis as any).__GEV_ENRICH_AMBIENT_QA;
    expect(qa.ceil).toBeGreaterThanOrEqual(800);
    expect(qa.refillTokens).toBeGreaterThanOrEqual(400);
    expect(qa.windowMs).toBe(300_000);
  });

  it("is idempotent (does not overwrite an existing override)", () => {
    (globalThis as any).__GEV_ENRICH_AMBIENT_QA = {
      ceil: 100,
      refillTokens: 50,
      windowMs: 60_000,
    };
    applyEnrichAmbientOverride();
    expect((globalThis as any).__GEV_ENRICH_AMBIENT_QA.ceil).toBe(100);
  });

  it("accepts caller overrides", () => {
    applyEnrichAmbientOverride({ ceil: 1500, refillTokens: 1000 });
    expect((globalThis as any).__GEV_ENRICH_AMBIENT_QA.ceil).toBe(1500);
    expect((globalThis as any).__GEV_ENRICH_AMBIENT_QA.refillTokens).toBe(1000);
    // Unspecified fields keep the shipped default.
    expect((globalThis as any).__GEV_ENRICH_AMBIENT_QA.windowMs).toBe(300_000);
  });
});
