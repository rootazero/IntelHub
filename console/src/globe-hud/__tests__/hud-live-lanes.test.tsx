// T14: live bottom-bar lanes — useActiveBasemap (engine mapStackController)
//   and useCursorCoordinates (Cesium ScreenSpaceEventHandler).
//   useActiveBasemap: initial read from the handle (the scene's first
//     setStack is silent — scene.js:113), live updates via the
//     'gev:map-stack-changed' window event, fallback label logic.
//   useCursorCoordinates: registers ONLY against a viewer with a pick
//     surface, MOUSE_MOVE → pickEllipsoid → degrees, off-globe pick clears,
//     cleanup removes the input action + destroys the handler.
import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import "@testing-library/jest-dom/vitest";

import {
  basemapLabelForId,
  readBasemapLabel,
  useActiveBasemap,
} from "../useActiveBasemap";
import type { BasemapStack } from "../useActiveBasemap";
import {
  formatCoord,
  useCursorCoordinates,
} from "../useCursorCoordinates";
import type { CursorCesium } from "../useCursorCoordinates";

afterEach(cleanup);

// ---- useActiveBasemap -------------------------------------------------------

function fakeStack(activeId: string | null): BasemapStack {
  return {
    getActiveId: () => activeId,
    getActiveStack: () =>
      activeId
        ? { id: activeId, label: `label-of-${activeId}` }
        : null,
  };
}

describe("useActiveBasemap", () => {
  test("null handle → null label (static fallback lane owns the chip)", () => {
    const { result } = renderHook(() => useActiveBasemap(null));
    expect(result.current).toBeNull();
  });

  test("initial label reads straight from the handle (silent first setStack)", () => {
    const { result } = renderHook(() =>
      useActiveBasemap(fakeStack("photoreal")),
    );
    expect(result.current).toBe("GOOGLE PHOTOREAL");
  });

  test("gev:map-stack-changed updates the label — incl. the esri fallback", () => {
    const stack = fakeStack("photoreal");
    const { result } = renderHook(() => useActiveBasemap(stack));
    expect(result.current).toBe("GOOGLE PHOTOREAL");
    act(() => {
      window.dispatchEvent(
        new CustomEvent("gev:map-stack-changed", {
          detail: { activeId: "esri-imagery", status: "error" },
        }),
      );
    });
    // Photoreal tile failure falls back to the keyless globe — the chip must
    // say so instead of keeping the stale GOOGLE PHOTOREAL guess.
    expect(result.current).toBe("ESRI IMAGERY");
  });

  test("event without activeId re-reads the handle", () => {
    const stack = fakeStack("esri-imagery");
    const { result } = renderHook(() => useActiveBasemap(stack));
    act(() => {
      window.dispatchEvent(new CustomEvent("gev:map-stack-changed"));
    });
    expect(result.current).toBe("ESRI IMAGERY");
  });

  test("unsubscribes on unmount", () => {
    const stack = fakeStack("photoreal");
    const { result, unmount } = renderHook(() => useActiveBasemap(stack));
    unmount();
    act(() => {
      window.dispatchEvent(
        new CustomEvent("gev:map-stack-changed", {
          detail: { activeId: "osm" },
        }),
      );
    });
    // No crash, and the unmounted hook does not hold state that matters.
    expect(result.current).toBe("GOOGLE PHOTOREAL");
  });

  test("pure resolvers: known ids, descriptor label, humanized fallback", () => {
    expect(basemapLabelForId("photoreal")).toBe("GOOGLE PHOTOREAL");
    expect(basemapLabelForId("esri-imagery")).toBe("ESRI IMAGERY");
    expect(basemapLabelForId("osm", fakeStack("osm"))).toBe("label-of-osm");
    expect(basemapLabelForId("bing-aerial", null)).toBe("BING AERIAL");
    expect(basemapLabelForId(null)).toBeNull();
    expect(readBasemapLabel(null)).toBeNull();
    expect(readBasemapLabel(fakeStack("photoreal"))).toBe("GOOGLE PHOTOREAL");
  });
});

// ---- useCursorCoordinates ---------------------------------------------------

/** Fake Cesium: captures the MOUSE_MOVE action, stubbed math. */
function fakeCesium() {
  const registered = {
    action: null as null | ((movement: { endPosition?: unknown }) => void),
    type: null as unknown,
  };
  const calls = { removeInputAction: 0, destroy: 0 };
  const cesium = {
    ScreenSpaceEventHandler: class {
      // T17 regression guard: the REAL Cesium constructor registers DOM
      // listeners on its argument (`element.addEventListener`) — passing a
      // scene object crashes the HUD tree at mount. The fake must reject a
      // canvas-less argument the same way.
      constructor(element?: unknown) {
        if (
          !element ||
          typeof (element as { addEventListener?: unknown }).addEventListener !== "function"
        )
          throw new TypeError("element.addEventListener is not a function");
      }
      setInputAction(action: (m: { endPosition?: unknown }) => void, type: unknown) {
        registered.action = action;
        registered.type = type;
      }
      removeInputAction(type: unknown) {
        if (type === registered.type) calls.removeInputAction++;
      }
      destroy() {
        calls.destroy++;
      }
    },
    ScreenSpaceEventType: { MOUSE_MOVE: "MOUSE_MOVE" },
    Cartographic: {
      fromCartesian: (c: { lat: number; lon: number }) => ({
        latitude: c.lat,
        longitude: c.lon,
      }),
    },
    Math: { toDegrees: (radians: number) => (radians * 180) / Math.PI },
  } satisfies CursorCesium;
  return { cesium, registered, calls };
}

/** Fake viewer: pickEllipsoid returns the position when it is a pick hit.
 * globe may be null — the hook must not throw (photoreal scenes hide the
 * globe, and a torn scene may not have it at all). */
function fakeViewer(hit: { lat: number; lon: number } | null) {
  return {
    scene: {
      canvas: { addEventListener: () => {}, removeEventListener: () => {} },
      camera: {
        pickEllipsoid: (_position: unknown, _ellipsoid: unknown) => hit,
      },
      globe: null,
    },
  };
}

describe("useCursorCoordinates", () => {
  // rAF control: the hook throttles setCoords to one update per frame, so
  // tests must drive frames manually.
  let rafQueue: Array<(() => void) | null>;
  let rafSeq: number;
  let cancelCount: number;

  beforeEach(() => {
    rafQueue = [];
    rafSeq = 0;
    cancelCount = 0;
    vi.stubGlobal("requestAnimationFrame", (cb: () => void) => {
      rafQueue.push(cb);
      return ++rafSeq;
    });
    vi.stubGlobal("cancelAnimationFrame", (id: number) => {
      if (rafQueue[id - 1]) {
        rafQueue[id - 1] = null;
        cancelCount++;
      }
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  function flushFrame() {
    const cb = rafQueue[0];
    rafQueue = [];
    act(() => cb?.());
  }

  test("no viewer handle → no handler, null coords", () => {
    const { cesium, calls } = fakeCesium();
    const { result } = renderHook(() =>
      useCursorCoordinates(null, cesium),
    );
    expect(result.current).toBeNull();
    expect(calls.destroy).toBe(0);
  });

  test("MOUSE_MOVE pick → signed degrees after the frame flushes; globe may be null", () => {
    const { cesium, registered } = fakeCesium();
    const viewer = fakeViewer({ lat: 0.5445, lon: 2.1203 }); // ≈31.23°N 121.47°E
    const { result } = renderHook(() =>
      useCursorCoordinates(viewer, cesium),
    );
    expect(registered.action).not.toBeNull();
    act(() => registered.action!({ endPosition: { x: 1, y: 1 } }));
    // Throttled: nothing lands until the pending frame runs.
    expect(result.current).toBeNull();
    flushFrame();
    expect(result.current).not.toBeNull();
    expect(result.current!.lat).toBeCloseTo(31.2, 0);
    expect(result.current!.lon).toBeCloseTo(121.47, 0);
  });

  test("rAF throttle: many moves per frame collapse to one setState with the latest pick", () => {
    const { cesium, registered } = fakeCesium();
    // Mutating hit proves only the LATEST move wins.
    let hit: { lat: number; lon: number } | null = { lat: 0.1, lon: 0.1 };
    const viewer = {
      scene: {
        canvas: { addEventListener: () => {}, removeEventListener: () => {} },
        camera: { pickEllipsoid: () => hit },
        globe: { ellipsoid: {} },
      },
    };
    const { result } = renderHook(() =>
      useCursorCoordinates(viewer, cesium),
    );
    for (let i = 0; i < 5; i++) {
      hit = { lat: i / 100, lon: i / 100 };
      act(() => registered.action!({ endPosition: { x: i, y: i } }));
    }
    expect(rafQueue.filter(Boolean)).toHaveLength(1); // ONE frame for 5 moves
    flushFrame();
    // The 5th (latest) pick wins: 0.04 rad → ≈2.2918° through toDegrees.
    expect(result.current!.lat).toBeCloseTo((0.04 * 180) / Math.PI, 3);
    // A move in the next frame schedules a fresh frame.
    act(() => registered.action!({ endPosition: { x: 9, y: 9 } }));
    expect(rafQueue.filter(Boolean)).toHaveLength(1);
    flushFrame();
    expect(result.current!.lat).toBeCloseTo((0.04 * 180) / Math.PI, 3);
  });

  test("off-globe pick clears the readout (throttled to the frame too)", () => {
    const { cesium, registered } = fakeCesium();
    let hit: { lat: number; lon: number } | null = { lat: 0.1, lon: 0.1 };
    const viewer = {
      scene: {
        canvas: { addEventListener: () => {}, removeEventListener: () => {} },
        camera: { pickEllipsoid: () => hit },
        globe: { ellipsoid: {} },
      },
    };
    const { result } = renderHook(() =>
      useCursorCoordinates(viewer, cesium),
    );
    act(() => registered.action!({ endPosition: { x: 1, y: 1 } }));
    flushFrame();
    expect(result.current).not.toBeNull();
    hit = null;
    act(() => registered.action!({ endPosition: { x: 2, y: 2 } }));
    flushFrame();
    expect(result.current).toBeNull();
  });

  test("unmount cancels the pending frame, removes MOUSE_MOVE and destroys the handler", () => {
    const { cesium, registered, calls } = fakeCesium();
    const viewer = fakeViewer({ lat: 0.1, lon: 0.2 });
    const { result, unmount } = renderHook(() =>
      useCursorCoordinates(viewer, cesium),
    );
    expect(registered.type).toBe("MOUSE_MOVE");
    act(() => registered.action!({ endPosition: { x: 1, y: 1 } }));
    expect(rafQueue.filter(Boolean)).toHaveLength(1);
    unmount();
    expect(cancelCount).toBe(1); // pending rAF cancelled — no post-unmount setState
    expect(calls.removeInputAction).toBe(1);
    expect(calls.destroy).toBe(1);
    expect(result.current).toBeNull();
  });

  test("formatCoord: 4 decimals, sign preserved", () => {
    expect(formatCoord(31.2304)).toBe("31.2304");
    expect(formatCoord(-78.958)).toBe("-78.9580");
  });
});
