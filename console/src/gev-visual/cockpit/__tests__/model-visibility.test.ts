// GEV P14 — cockpit model-visibility tests.
//
// Hides the tracked aircraft model so the cockpit frame is empty sky + HUD
// (not "plane in the middle + cockpit chrome"). Asserts enter→exit round-trip
// restores the prior visibility value (defensive: don't clobber user state).

import { describe, expect, it, vi } from "vitest";
import { mountModelVisibility } from "../model-visibility";

type FakeEntity = { show: boolean; id: string };

function makeStore(initial: { active: boolean; trackedId: string | null }) {
  const listeners = new Set<() => void>();
  let state = initial;
  return {
    getState: () => state,
    subscribe: (fn: () => void) => {
      listeners.add(fn);
      return () => listeners.delete(fn);
    },
    _set: (next: typeof state) => {
      state = next;
      listeners.forEach((fn) => fn());
    },
  };
}

const DUMMY = { id: "icao-1" };

describe("mountModelVisibility", () => {
  it("throws TypeError if store.subscribe is missing", () => {
    expect(() =>
      mountModelVisibility({
        store: { getState: () => ({ active: false, trackedId: null }) } as never,
        getTrackedEntity: () => null,
      }),
    ).toThrow(TypeError);
  });

  it("hides the entity on cockpit enter and restores on exit", () => {
    const entity: FakeEntity = { show: true, id: "icao-1" };
    const store = makeStore({ active: false, trackedId: null });
    const handle = mountModelVisibility({
      store,
      getTrackedEntity: () => entity as never,
    });
    expect(entity.show).toBe(true); // baseline untouched before enter

    store._set({ active: true, trackedId: "icao-1" });
    expect(entity.show).toBe(false); // hidden

    store._set({ active: false, trackedId: null });
    expect(entity.show).toBe(true); // restored

    handle.destroy();
  });

  it("does not crash when entity is null at enter time", () => {
    const store = makeStore({ active: false, trackedId: null });
    const handle = mountModelVisibility({
      store,
      getTrackedEntity: () => null,
    });
    expect(() => store._set({ active: true, trackedId: "icao-1" })).not.toThrow();
    expect(() => store._set({ active: false, trackedId: null })).not.toThrow();
    handle.destroy();
  });

  it("preserves prior show=false (does not overwrite a hidden plane)", () => {
    const entity: FakeEntity = { show: false, id: "icao-1" };
    const store = makeStore({ active: false, trackedId: null });
    const handle = mountModelVisibility({
      store,
      getTrackedEntity: () => entity as never,
    });
    store._set({ active: true, trackedId: "icao-1" });
    expect(entity.show).toBe(false); // still false
    store._set({ active: false, trackedId: null });
    expect(entity.show).toBe(false); // restored to false, not true
    handle.destroy();
  });

  it("destroy() stops subscribing; later store updates do not toggle show", () => {
    const entity: FakeEntity = { show: true, id: "icao-1" };
    const store = makeStore({ active: false, trackedId: null });
    const handle = mountModelVisibility({
      store,
      getTrackedEntity: () => entity as never,
    });
    handle.destroy();
    store._set({ active: true, trackedId: "icao-1" });
    expect(entity.show).toBe(true); // no toggle after destroy
  });
});
