// humanizeKind test suite (§FE-RADAR-i18n + §FE-RADAR-PRIMARY-CATEGORIES).
// Pins the 3-tier fallback contract for both L2 (granular kinds) and
// L1 (primary display categories) surfaces:
//   1. active-language dict
//   2. English fallback (when lang != en)
//   3. TitleCase of snake_case id
// Plus a knownKinds() / knownPrimaryKinds() smoke check — Radar's
// dynamic dropdown relies on it to seed its option list.

import { describe, it, expect } from "vitest";
import {
  humanizeKind,
  humanizeL1,
  titleCaseKind,
  knownKinds,
  knownPrimaryKinds,
} from "../humanizeKind";

describe("humanizeKind (L2 granular)", () => {
  it("returns the active-language label", () => {
    expect(humanizeKind("conflict", "en")).toBe("conflict");
    // zh has its own translation for conflict
    expect(humanizeKind("conflict", "zh")).toBe("冲突");
  });

  it("returns the active-language label for all configured kinds", () => {
    // `holiday` is i18n'd in both dicts.
    expect(humanizeKind("holiday", "en")).toBe("holiday");
    expect(humanizeKind("holiday", "zh")).toBe("节假日");
  });

  it("falls back to TitleCase when neither dict has the key", () => {
    // Synthesize a key that's definitely not i18n'd anywhere.
    expect(humanizeKind("future_kind_added_in_a_later_release", "en")).toBe(
      "Future Kind Added In A Later Release",
    );
    expect(humanizeKind("future_kind_added_in_a_later_release", "zh")).toBe(
      "Future Kind Added In A Later Release",
    );
  });

  it("returns '' for empty input", () => {
    expect(humanizeKind("", "en")).toBe("");
    expect(humanizeKind("", "zh")).toBe("");
  });

  it("uses the configured i18n entry for newly-added dynamic kinds", () => {
    // These entries are added by the §FE-RADAR-i18n patch.
    expect(humanizeKind("festival_holiday_imminent_priority", "en")).toBe(
      "Imminent Holiday (Priority)",
    );
    expect(humanizeKind("festival_holiday_imminent_priority", "zh")).toBe(
      "临近节日 (优先)",
    );
    expect(humanizeKind("fire_extreme", "en")).toBe("Extreme Fire");
    expect(humanizeKind("fire_extreme", "zh")).toBe("极端火点");
  });

  it("resolves HDX HAPI kinds to L2 labels added by §FE-RADAR-PRIMARY-CATEGORIES", () => {
    expect(humanizeKind("hdx_hapi_national_risk_priority", "en")).toBe(
      "HDX HAPI National Risk (Priority)",
    );
    expect(humanizeKind("hdx_hapi_national_risk_priority", "zh")).toBe(
      "HDX HAPI 国家风险 (优先)",
    );
    expect(humanizeKind("hdx_hapi_funding_info", "en")).toBe(
      "HDX HAPI Funding (Info)",
    );
  });
});

describe("humanizeL1 (primary display)", () => {
  it("returns the active-language label for each L1 category", () => {
    expect(humanizeL1("conflict", "en")).toBe("Conflict");
    expect(humanizeL1("conflict", "zh")).toBe("冲突");
    expect(humanizeL1("disaster", "en")).toBe("Disaster");
    expect(humanizeL1("disaster", "zh")).toBe("灾害");
  });

  it("returns '' for empty input", () => {
    expect(humanizeL1("", "en")).toBe("");
    expect(humanizeL1("", "zh")).toBe("");
  });

  it("falls back to English when active language lacks the key", () => {
    // Synthetic case — should fall through to en dict.
    // (All 15 L1 keys are present in both dicts in practice.)
    expect(humanizeL1("climate", "zh")).toBe("气候");
    expect(humanizeL1("climate", "en")).toBe("Climate");
  });

  it("falls back to TitleCase for unknown L1 keys", () => {
    expect(humanizeL1("newly_added_primary", "en")).toBe(
      "Newly Added Primary",
    );
    expect(humanizeL1("newly_added_primary", "zh")).toBe(
      "Newly Added Primary",
    );
  });

  it("L1 labels are distinct from L2 labels (no name collisions in dict)", () => {
    // Sanity: humanizeL1 and humanizeKind for the same key return
    // different strings — the L1 dict uses TitleCase-style labels
    // ("Conflict") while the L2 dict keeps the snake_case id
    // ("conflict"). This separation is what allows Radar to show
    // "Conflict" on the chip while the detail drawer still shows
    // "conflict" for the precise L2 kind.
    expect(humanizeL1("conflict", "en")).not.toBe(humanizeKind("conflict", "en"));
  });
});

describe("titleCaseKind", () => {
  it("TitleCases the common snake_case id", () => {
    expect(titleCaseKind("festival_holiday_past")).toBe(
      "Festival Holiday Past",
    );
  });
  it("handles kebab-case too", () => {
    expect(titleCaseKind("news-headline")).toBe("News Headline");
  });
  it("collapses repeated separators", () => {
    expect(titleCaseKind("foo__bar")).toBe("Foo Bar");
    expect(titleCaseKind("foo - bar")).toBe("Foo Bar");
  });
  it("handles single words", () => {
    expect(titleCaseKind("quake")).toBe("Quake");
  });
  it("returns empty string for empty input", () => {
    expect(titleCaseKind("")).toBe("");
  });
});

describe("knownKinds (L2)", () => {
  it("returns every key in enum.kind (L2)", () => {
    const kinds = knownKinds();
    // The §FE-RADAR-i18n patch adds 23 dynamic kinds on top of the
    // 17 static ones, plus ~33 more from §FE-RADAR-PRIMARY-CATEGORIES
    // (HDX HAPI, arcnautical, threat cluster, secret leak, etc.).
    // Pin to a generous floor so any future removal trips the test.
    expect(kinds.length).toBeGreaterThanOrEqual(40);
    // Sorted alphabetically for stable dropdown order.
    const sorted = [...kinds].sort();
    expect(kinds).toEqual(sorted);
    // Sanity-check that known entries are present.
    expect(kinds).toContain("conflict");
    expect(kinds).toContain("festival_holiday_past");
    expect(kinds).toContain("festival_holiday_imminent_priority");
    expect(kinds).toContain("hdx_hapi_national_risk_priority");
  });
});

describe("knownPrimaryKinds (L1)", () => {
  it("returns exactly 15 L1 categories", () => {
    const kinds = knownPrimaryKinds();
    expect(kinds).toHaveLength(15);
    // Sorted alphabetically for stable display order.
    const sorted = [...kinds].sort();
    expect(kinds).toEqual(sorted);
  });

  it("includes every OSINT-standard L1 category", () => {
    const kinds = knownPrimaryKinds();
    for (const expected of [
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
    ]) {
      expect(kinds).toContain(expected);
    }
  });
});