// GEV P13 T1 — default Cesium camera view.
//
// The failure this pins: the IntelHub boot never runs the vendor's
// `flyToAustin` (application.ts does not call createApplicationControls), so
// the globe kept Cesium's built-in US-rectangle default and operators read the
// aircraft field as a regional (Florida-only) picture. `mountDefaultCamera`
// forces the global Western-hemisphere overview instead.
//
// The constructor-contract test is the P3 lesson: a lenient fake viewer must
// not hide a seam the module depends on (a viewer without camera.setView).
import { describe, expect, it, vi } from "vitest";
import * as Cesium from "cesium";
import { DEFAULT_VIEW, mountDefaultCamera } from "../default-camera";

describe("mountDefaultCamera", () => {
  it("throws TypeError if viewer.camera.setView is missing", () => {
    expect(() => mountDefaultCamera({} as any)).toThrow(TypeError);
  });

  it("calls viewer.camera.setView with DEFAULT_VIEW destination + pitch", () => {
    // The constants are the spec (plan §Task 1) — pin them so a refactor
    // cannot silently drift the view back to a regional framing.
    expect(DEFAULT_VIEW).toEqual({
      lon: -50,
      lat: 20,
      alt: 12_000_000,
      pitch: -90,
    });
    const setView = vi.fn();
    mountDefaultCamera({ camera: { setView } } as any);
    expect(setView).toHaveBeenCalledTimes(1);
    const arg = setView.mock.calls[0][0];
    expect(arg.orientation.pitch).toBeCloseTo(-Math.PI / 2);
    expect(arg.destination.x).toBeGreaterThan(0); // Cartesian3.fromDegrees(...)
    expect(
      Cesium.Cartesian3.equals(
        arg.destination,
        Cesium.Cartesian3.fromDegrees(
          DEFAULT_VIEW.lon,
          DEFAULT_VIEW.lat,
          DEFAULT_VIEW.alt,
        ),
      ),
    ).toBe(true);
  });
});
