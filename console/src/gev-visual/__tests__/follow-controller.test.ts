import { describe, expect, test, vi } from "vitest";
import { mountFollowController } from "../follow-controller";

function fakeDataManager(opts: { flightsTrackOk?: boolean; satsTrackOk?: boolean } = {}) {
  const flightsTrackOk = opts.flightsTrackOk ?? true;
  const satsTrackOk = opts.satsTrackOk ?? true;
  const flightsModule = {
    trackById: vi.fn((id: unknown, _opts?: unknown) => {
      if (typeof id !== "string") throw new TypeError("flights.trackById: icao24 must be string");
      return flightsTrackOk;
    }),
    stopTracking: vi.fn(() => true),
    getTrackedInfo: vi.fn(() => ({ icao24: "abc123" })),
  };
  const satellitesModule = {
    trackById: vi.fn((id: unknown) => {
      if (typeof id !== "number") throw new TypeError("satellites.trackById: noradId must be number");
      return satsTrackOk;
    }),
    stopTracking: vi.fn(() => true),
    getTrackedInfo: vi.fn(() => null),
  };
  const layers = new Map([
    ["flights", { module: flightsModule }],
    ["satellites", { module: satellitesModule }],
  ]);
  return { layers, flightsModule, satellitesModule };
}

describe("mountFollowController", () => {
  test("constructor contract: rejects dataManager without layers Map", () => {
    expect(() => mountFollowController({} as any)).toThrow(TypeError);
  });

  test("follow flight tracks via flights module with string icao24", () => {
    const dm = fakeDataManager();
    const h = mountFollowController(dm as any);
    expect(h.follow("flight", "abc123")).toBe(true);
    expect(dm.flightsModule.trackById).toHaveBeenCalledWith("abc123", { origin: "programmatic" });
    expect(h.trackedId()).toBe("abc123");
  });

  test("follow satellite converts noradId to number", () => {
    const dm = fakeDataManager();
    const h = mountFollowController(dm as any);
    expect(h.follow("satellite", "25544")).toBe(true);
    expect(dm.satellitesModule.trackById).toHaveBeenCalledWith(25544, { origin: "programmatic" });
  });

  test("follow returns false when layer missing or trackById declines", () => {
    const dm = fakeDataManager({ flightsTrackOk: false });
    const h = mountFollowController(dm as any);
    expect(h.follow("flight", "abc123")).toBe(false);
    expect(h.trackedId()).toBeNull();
    dm.layers.delete("flights");
    expect(h.follow("flight", "abc123")).toBe(false);
  });

  test("unfollow stops tracking on the owning layer and clears trackedId", () => {
    const dm = fakeDataManager();
    const h = mountFollowController(dm as any);
    h.follow("flight", "abc123");
    h.unfollow();
    expect(dm.flightsModule.stopTracking).toHaveBeenCalledWith({ origin: "programmatic" });
    expect(h.trackedId()).toBeNull();
  });

  test("follow switches layers: previous layer stopTracking called", () => {
    const dm = fakeDataManager();
    const h = mountFollowController(dm as any);
    h.follow("flight", "abc123");
    h.follow("satellite", "25544");
    expect(dm.flightsModule.stopTracking).toHaveBeenCalled();
    expect(h.trackedId()).toBe("25544");
  });
});
