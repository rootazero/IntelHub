import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import {
  COCKPIT_ELEMENT_KEYS,
  DEFAULT_ELEMENT_VISIBILITY,
  ELEMENT_VISIBILITY_STORAGE_KEY,
  isCockpitElementKey,
  persistElementVisibility,
  readPersistedElementVisibility,
  type ElementVisibility,
} from "../element-visibility";

describe("element-visibility", () => {
  beforeEach(() => {
    localStorage.clear();
  });
  afterEach(() => {
    localStorage.clear();
    vi.restoreAllMocks();
  });

  test("exports 8 element keys", () => {
    expect(COCKPIT_ELEMENT_KEYS).toHaveLength(8);
    expect(COCKPIT_ELEMENT_KEYS).toContain("compass");
    expect(COCKPIT_ELEMENT_KEYS).toContain("vsiChevron");
  });

  test("DEFAULT_ELEMENT_VISIBILITY is all true", () => {
    for (const k of COCKPIT_ELEMENT_KEYS) {
      expect(DEFAULT_ELEMENT_VISIBILITY[k]).toBe(true);
    }
  });

  test("storage key is intelhub.cockpit.elementVisibility", () => {
    expect(ELEMENT_VISIBILITY_STORAGE_KEY).toBe(
      "intelhub.cockpit.elementVisibility",
    );
  });

  test("readPersistedElementVisibility returns defaults on empty storage", () => {
    expect(readPersistedElementVisibility()).toEqual(
      DEFAULT_ELEMENT_VISIBILITY,
    );
  });

  test("readPersistedElementVisibility returns defaults on invalid JSON", () => {
    localStorage.setItem(ELEMENT_VISIBILITY_STORAGE_KEY, "{not json");
    expect(readPersistedElementVisibility()).toEqual(
      DEFAULT_ELEMENT_VISIBILITY,
    );
  });

  test("readPersistedElementVisibility merges partial saved state with defaults", () => {
    const saved: Partial<ElementVisibility> = { speedTape: false };
    localStorage.setItem(
      ELEMENT_VISIBILITY_STORAGE_KEY,
      JSON.stringify(saved),
    );
    const result = readPersistedElementVisibility();
    expect(result.speedTape).toBe(false);
    expect(result.compass).toBe(true);
  });

  test("readPersistedElementVisibility drops stale keys", () => {
    localStorage.setItem(
      ELEMENT_VISIBILITY_STORAGE_KEY,
      JSON.stringify({ compass: false, retiredKey: false }),
    );
    const result = readPersistedElementVisibility();
    expect(result.compass).toBe(false);
    expect((result as Record<string, unknown>).retiredKey).toBeUndefined();
  });

  test("readPersistedElementVisibility drops non-boolean values", () => {
    localStorage.setItem(
      ELEMENT_VISIBILITY_STORAGE_KEY,
      JSON.stringify({ compass: "yes", speedTape: 0 }),
    );
    const result = readPersistedElementVisibility();
    expect(result.compass).toBe(true); // not a bool → default
    expect(result.speedTape).toBe(true); // 0 is not boolean → default
  });

  test("readPersistedElementVisibility returns defaults when localStorage throws", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("access denied");
    });
    expect(readPersistedElementVisibility()).toEqual(
      DEFAULT_ELEMENT_VISIBILITY,
    );
  });

  test("persistElementVisibility round-trips through read", () => {
    const next: ElementVisibility = {
      ...DEFAULT_ELEMENT_VISIBILITY,
      pitchLadder: false,
    };
    persistElementVisibility(next);
    expect(readPersistedElementVisibility().pitchLadder).toBe(false);
  });

  test("persistElementVisibility swallows storage errors", () => {
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("QuotaExceeded");
    });
    expect(() =>
      persistElementVisibility(DEFAULT_ELEMENT_VISIBILITY),
    ).not.toThrow();
  });

  test("isCockpitElementKey narrows correctly", () => {
    expect(isCockpitElementKey("compass")).toBe(true);
    expect(isCockpitElementKey("vsiChevron")).toBe(true);
    expect(isCockpitElementKey("retiredKey")).toBe(false);
    expect(isCockpitElementKey(123)).toBe(false);
    expect(isCockpitElementKey(null)).toBe(false);
    expect(isCockpitElementKey(undefined)).toBe(false);
  });
});