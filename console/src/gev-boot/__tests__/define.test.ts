// Regression guard for the vitest config trap: when console/vitest.config.ts
// exists, vitest IGNORES vite.config.ts entirely — so the engine import alias
// and the import.meta.env.* define keys must be duplicated inside
// vitest.config.ts. If someone deletes them, this test fails.
import { describe, expect, it } from "vitest";

describe("vitest define parity with vite.config.ts", () => {
  it("GOOGLE_MAPS_API_KEY is defined (not undefined)", () => {
    expect(typeof import.meta.env.GOOGLE_MAPS_API_KEY).not.toBe("undefined");
  });
  it("CESIUM_ION_TOKEN is defined (not undefined)", () => {
    expect(typeof import.meta.env.CESIUM_ION_TOKEN).not.toBe("undefined");
  });
});
