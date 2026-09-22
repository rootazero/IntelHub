// GEV P19 SVS — Cesium ion token initialization.
//
// Sets `Cesium.Ion.defaultAccessToken` as early as possible so any
// Cesium ion asset request (terrain, Bing imagery) authenticates
// with the user's ion account. The token comes from the build-time
// env (`VITE_CESIUM_ION_KEY`); without it, Cesium falls back to its
// rate-limited public token and SVS / 3D tiles degrade.
//
// Idempotent. Safe to call multiple times. No-op if Cesium isn't
// yet imported (the bundled module is the only Cesium we touch).
//
// Per §6.3 spec (D-SVS-2=B): Cesium-native terrain — we don't
// instantiate a new provider here, just set the token so the engine's
// own globe setup can use it.

import * as Cesium from "cesium";
import { CESIUM_ION_KEY } from "./basemap";

let initialized = false;

export function initCesiumIon(): boolean {
  if (initialized) return true;
  if (!CESIUM_ION_KEY) {
    // No token → use Cesium's public token (rate-limited). Logged once
    // so operators notice; not a hard failure.
    if (typeof console !== "undefined") {
      console.warn(
        "[cesium-init] VITE_CESIUM_ION_KEY not set — using Cesium's public token (rate-limited). Sign up free at https://ion.cesium.com/ and set VITE_CESIUM_ION_KEY in core/console-build.env.",
      );
    }
  } else {
    Cesium.Ion.defaultAccessToken = CESIUM_ION_KEY;
  }
  initialized = true;
  return true;
}

/** Test-only reset hook. */
export function _resetCesiumIonForTest(): void {
  initialized = false;
  Cesium.Ion.defaultAccessToken = "";
}