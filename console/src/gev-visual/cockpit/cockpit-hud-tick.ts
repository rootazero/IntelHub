// GEV P16 T3 — cockpit HUD tick driver. Owns a 10Hz interval that drains
// flights.getTrackedInfo() + chaseCam.getResolvedState() into the cockpit
// instruments adapter on every frame. The vendored cockpit HUD historically
// ran its own requestAnimationFrame loop, but the React HUD (T4) reads a
// pure frame, so this adapter is the *only* cadence the cockpit depends on.
//
// Lifecycle:
//   - start() — begin the interval (idempotent: double-start is a noop).
//   - stop()  — cancel and reset internal handle.
//   - isRunning() — reports whether the interval is armed.
// On every tick we re-check options.viewer?.isDestroyed?.() so a Cesium
// teardown never leaks tick calls into a destroyed scene (P3 lesson — the
// HUD must not call viewer APIs after disposal).
import type { InstrumentsHandle } from "./instruments-mount";
import type { ChaseCamResolvedState } from "./chase-cam";
import type { CockpitTrackedInfo } from "./instruments-mount";

export interface HudTickOptions {
  /** Cadence in ms. Default 100 → 10Hz. */
  intervalMs?: number;
  /** Optional Cesium-shaped viewer. When provided, ticks short-circuit on
   *  viewer.isDestroyed?.() and the interval self-clears. */
  viewer?: { isDestroyed(): boolean };
}

export interface HudTickHandle {
  start(): void;
  stop(): void;
  isRunning(): boolean;
}

/** Shape chase-cam must satisfy to feed the tick. Loosely typed because the
 *  chase-cam module owns its own resolution semantics; we only need a getter. */
export interface CockpitTickChaseCam {
  getResolvedState(): ChaseCamResolvedState | null;
}

/** Shape flights must satisfy to feed the tick. */
export interface CockpitTickFlights {
  getTrackedInfo(): CockpitTrackedInfo | null;
}

export function mountCockpitHudTick(
  instruments: InstrumentsHandle,
  chaseCam: CockpitTickChaseCam,
  flights: CockpitTickFlights,
  options: HudTickOptions = {},
): HudTickHandle {
  const intervalMs = options.intervalMs ?? 100;
  let handle: ReturnType<typeof setInterval> | null = null;

  function tick() {
    // Defensive: a Cesium viewer may have been disposed by the host between
    // ticks (mode switch, scene rebuild). Short-circuit + self-clear so we
    // don't keep calling into a destroyed scene.
    if (options.viewer?.isDestroyed?.()) {
      if (handle) clearInterval(handle);
      handle = null;
      return;
    }
    const info = flights.getTrackedInfo();
    const resolved = chaseCam.getResolvedState();
    instruments.update(info, resolved);
  }

  return {
    start() {
      if (handle) return;
      handle = setInterval(tick, intervalMs);
    },
    stop() {
      if (handle) {
        clearInterval(handle);
        handle = null;
      }
    },
    isRunning: () => handle !== null,
  };
}