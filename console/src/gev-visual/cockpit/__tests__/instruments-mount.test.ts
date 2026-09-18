import { describe, expect, test, vi } from "vitest";
import { mountCockpitInstruments } from "../instruments-mount";
import type { CockpitTrackedInfo } from "../instruments-mount";

// The adapter imports cockpitMath.js for REAL (pure module, no Cesium/DOM —
// T1 live-imports it in source-contracts), so these tests assert the real math
// end-to-end. Only the viewer + flights seams are mocked, and the mock
// replicates the REAL constructor contracts (P3 lesson):
//   - viewer.scene.canvas must exist (the Cesium render seam the cockpit uses)
//   - flights.getTrackedInfo() returns the exact
//     layers/flights/queries.js shape {icao24, callsign, latitude, longitude,
//     altitudeM, velocityMps, track, onGround, registration, layerId, stale}
function fakeViewer() {
  return { scene: { canvas: {} } };
}

const trackedInfo: CockpitTrackedInfo = {
  icao24: "abc123",
  callsign: "UAL123",
  registration: null,
  latitude: 39.9,
  longitude: 116.4,
  altitudeM: 1000,
  velocityMps: 100,
  track: 90,
  onGround: false,
  layerId: "flights",
  stale: false,
};

function fakeFlights(info: CockpitTrackedInfo | null = trackedInfo) {
  return { getTrackedInfo: vi.fn(() => info) };
}

describe("mountCockpitInstruments", () => {
  test("constructor contract: rejects a viewer without scene.canvas", () => {
    expect(() =>
      mountCockpitInstruments({ scene: {} } as any),
    ).toThrow(TypeError);
    expect(() => mountCockpitInstruments({} as any)).toThrow(TypeError);
  });

  test("update() with no argument pulls flights.getTrackedInfo()", () => {
    const flights = fakeFlights();
    const h = mountCockpitInstruments(fakeViewer() as any, flights as any);
    const frame = h.update();
    expect(flights.getTrackedInfo).toHaveBeenCalled();
    expect(frame.callsign).toBe("UAL123");
    expect(frame.heading).toBe(90);
  });

  test("update(info) converts track→heading, m/s→knots, altitudeM→feet", () => {
    const h = mountCockpitInstruments(fakeViewer() as any);
    const frame = h.update(trackedInfo);
    expect(frame.heading).toBe(90);
    expect(frame.headingLabel).toBe("090");
    expect(frame.speedKt).toBeCloseTo(100 * 1.94384, 4);
    expect(frame.speedLabel).toBe("194"); // Math.round(194.384).padStart(3,'0')
    expect(frame.altitudeFt).toBeCloseTo(1000 * 3.28084, 4);
    expect(frame.altitudeLabel).toBe("3,281"); // toLocaleString('en-US')
    expect(frame.callsign).toBe("UAL123");
  });

  test("compass divisions center on the heading with the active slot at index 3", () => {
    const h = mountCockpitInstruments(fakeViewer() as any);
    const frame = h.update(trackedInfo);
    expect(frame.compass).toHaveLength(7);
    expect(frame.compass[3].active).toBe(true);
    expect(frame.compass[3].label).toBe("E"); // heading 90 → division 90
    expect(frame.compass[0].division).toBe(0);
    expect(frame.compass[0].label).toBe("N");
    expect(frame.compass[6].division).toBe(180);
    expect(frame.compass[6].label).toBe("S");
  });

  test("altitude + speed ruler ticks have 9 slots with major/label/curve", () => {
    const h = mountCockpitInstruments(fakeViewer() as any);
    const frame = h.update(trackedInfo);
    expect(frame.altitudeTicks).toHaveLength(9);
    expect(frame.speedTicks).toHaveLength(9);
    // altitude step is 100 ft below 5000 ft; the 3200 ft tick is major.
    const tick3200 = frame.altitudeTicks.find((t) => t.value === 3200);
    expect(tick3200).toBeTruthy();
    expect(tick3200!.major).toBe(true);
    expect(tick3200!.label).toBe("03200");
    expect(tick3200!.curve).toBeGreaterThanOrEqual(0);
    // speed step is 20 kt in [100,300); 200 kt tick is major.
    const tick200 = frame.speedTicks.find((t) => t.value === 200);
    expect(tick200).toBeTruthy();
    expect(tick200!.major).toBe(true);
    expect(tick200!.label).toBe("200");
  });

  test("onGround=true reads zero feet (vendor cockpitAltitudeDisplayFt)", () => {
    const h = mountCockpitInstruments(fakeViewer() as any);
    const frame = h.update({ ...trackedInfo, onGround: true, altitudeM: 1200 });
    expect(frame.altitudeFt).toBe(0);
    expect(frame.altitudeLabel).toBe("0");
  });

  test("null info yields a neutral dashed frame, never throws", () => {
    const h = mountCockpitInstruments(fakeViewer() as any);
    const frame = h.update(null);
    expect(frame.callsign).toBe("AIRCRAFT");
    expect(frame.altitudeFt).toBeNull();
    expect(frame.altitudeLabel).toBe("-----");
    expect(frame.speedKt).toBeNull();
    expect(frame.speedLabel).toBe("---");
    expect(frame.altitudeTicks).toEqual([]);
    expect(frame.speedTicks).toEqual([]);
    expect(frame.compass).toHaveLength(7);
  });

  test("destroy clears the frame and is idempotent", () => {
    const h = mountCockpitInstruments(fakeViewer() as any);
    h.update(trackedInfo);
    expect(h.getFrame()).not.toBeNull();
    h.destroy();
    expect(h.getFrame()).toBeNull();
    expect(() => h.destroy()).not.toThrow();
  });
});
