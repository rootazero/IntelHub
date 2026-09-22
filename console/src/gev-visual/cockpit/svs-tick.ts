// GEV P19 SVS — periodic terrain sampling tick.
//
// Per §6.3 spec (D-SVS-1=A): wireframe overlay driven from a
// Cesium-native terrain sampler. The tick fires every
// `SAMPLE_INTERVAL_MS` (default 1000 ms = 1 Hz) when SVS is enabled;
// samples are passed to subscribers via the callback.
//
// Pure adapter (mount + tick + destroy). No DOM, no Cesium scene
// mutation. The HUD overlay component is the subscriber.

import {
  mountCockpitTerrainSampler,
  type CockpitTerrainSamplerHandle,
  type SvsSamplePoint,
} from "./svs-terrain-sampler";

const DEFAULT_SAMPLE_INTERVAL_MS = 1000;

export interface SvsTickDeps {
  /** Live terrain sampler handle (built by the caller). */
  sampler: CockpitTerrainSamplerHandle;
  /** Cadence in ms (default 1000). */
  intervalMs?: number;
}

export interface SvsTickHandle {
  start(onSamples: (samples: SvsSamplePoint[]) => void): void;
  stop(): void;
  destroy(): void;
}

export function mountCockpitSvsTick(deps: SvsTickDeps): SvsTickHandle {
  const intervalMs = deps.intervalMs ?? DEFAULT_SAMPLE_INTERVAL_MS;
  let timer: ReturnType<typeof setInterval> | null = null;
  let callback: ((samples: SvsSamplePoint[]) => void) | null = null;
  let destroyed = false;

  function tick() {
    if (destroyed || !callback) return;
    deps.sampler
      .sample()
      .then((samples) => {
        if (destroyed || !callback) return;
        callback(samples);
      })
      .catch(() => {
        /* swallow — quota / provider errors don't break the cockpit */
      });
  }

  return {
    start(onSamples) {
      if (destroyed) return;
      callback = onSamples;
      if (timer !== null) return;
      // First sample immediately so the overlay appears without a 1s
      // blank flash when the user toggles SVS on.
      tick();
      timer = setInterval(tick, intervalMs);
    },
    stop() {
      if (timer !== null) {
        clearInterval(timer);
        timer = null;
      }
      callback = null;
    },
    destroy() {
      destroyed = true;
      if (timer !== null) {
        clearInterval(timer);
        timer = null;
      }
      callback = null;
    },
  };
}