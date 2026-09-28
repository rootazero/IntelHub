// humanizeKind test suite (§FE-RADAR-i18n).
// Pins the 3-tier fallback contract:
//   1. active-language dict
//   2. English fallback (when lang != en)
//   3. TitleCase of snake_case id
// Plus a knownKinds() smoke check — Radar's dynamic dropdown relies
// on it to seed its option list.

import { describe, it, expect } from "vitest";
import { humanizeKind, titleCaseKind, knownKinds } from "../humanizeKind";

describe("humanizeKind", () => {
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

describe("knownKinds", () => {
  it("returns every key in enum.kind", () => {
    const kinds = knownKinds();
    // The §FE-RADAR-i18n patch adds 23 dynamic kinds on top of the
    // 17 static ones, for a total of at least 40. Pin to that
    // floor so any future removal trips the test.
    expect(kinds.length).toBeGreaterThanOrEqual(40);
    // Sorted alphabetically for stable dropdown order.
    const sorted = [...kinds].sort();
    expect(kinds).toEqual(sorted);
    // Sanity-check that known entries are present.
    expect(kinds).toContain("conflict");
    expect(kinds).toContain("festival_holiday_past");
    expect(kinds).toContain("festival_holiday_imminent_priority");
  });
});