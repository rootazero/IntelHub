// GEV §6.3 Replay player — drives the cockpit-store at scrub
// position. Pure DOM-less adapter; HUD subscribes to its onTick
// callback to mirror state into the cockpit frame.

import type { ReplayFrame, ReplaySegment } from "./replay-types";
import { DEFAULT_SAMPLE_INTERVAL_MS } from "./replay-types";

export interface ReplayPlayerHandle {
  load(segment: ReplaySegment): Promise<void>;
  /** Returns the frame nearest to tMs (with linear interpolation
   *  between adjacent frames), or null if tMs is out of range. */
  seek(tMs: number): ReplayFrame | null;
  play(opts: { speed: number; fromMs?: number }): void;
  pause(): void;
  setSpeed(speed: number): void;
  currentSegment(): ReplaySegment | null;
  currentTimeMs(): number;
  isPlaying(): boolean;
  destroy(): void;
}

export interface MountCockpitReplayPlayerDeps {
  /** Called every tick during play. The HUD uses this to mirror
   *  the replayed frame into the cockpit instruments. */
  onTick?: (frame: ReplayFrame) => void;
}

/** Linear interpolation between two frames at parameter t ∈ [0,1].
 *  Heading wraps so we pick the shortest angular path. */
function lerpFrame(a: ReplayFrame, b: ReplayFrame, t: number): ReplayFrame {
  const dh = ((b.frame.heading - a.frame.heading + 540) % 360) - 180;
  const heading = ((a.frame.heading + dh * t) % 360 + 360) % 360;
  return {
    tMs: a.tMs + (b.tMs - a.tMs) * t,
    frame: {
      heading,
      pitchRad:
        a.frame.pitchRad + (b.frame.pitchRad - a.frame.pitchRad) * t,
      bankRad:
        a.frame.bankRad + (b.frame.bankRad - a.frame.bankRad) * t,
      altitudeFt:
        a.frame.altitudeFt != null && b.frame.altitudeFt != null
          ? a.frame.altitudeFt +
            (b.frame.altitudeFt - a.frame.altitudeFt) * t
          : null,
      speedKt:
        a.frame.speedKt != null && b.frame.speedKt != null
          ? a.frame.speedKt + (b.frame.speedKt - a.frame.speedKt) * t
          : null,
      vsiMps: a.frame.vsiMps + (b.frame.vsiMps - a.frame.vsiMps) * t,
      callsign: a.frame.callsign, // discrete field, no interpolation
    },
  };
}

export function mountCockpitReplayPlayer(
  deps: MountCockpitReplayPlayerDeps,
): ReplayPlayerHandle {
  let segment: ReplaySegment | null = null;
  let currentMs = 0;
  let speed = 1;
  let intervalId: ReturnType<typeof setInterval> | null = null;

  function tickFrame() {
    if (!segment) return;
    currentMs += DEFAULT_SAMPLE_INTERVAL_MS * speed;
    if (currentMs >= segment.durationMs) {
      handle.pause();
      return;
    }
    const f = handle.seek(currentMs);
    if (f && deps.onTick) deps.onTick(f);
  }

  const handle: ReplayPlayerHandle = {
    async load(seg: ReplaySegment) {
      segment = seg;
      currentMs = 0;
    },

    seek(tMs: number) {
      if (!segment || segment.frames.length === 0) return null;
      if (tMs < 0 || tMs > segment.durationMs) return null;
      const frames = segment.frames;
      let lo = 0;
      let hi = frames.length - 1;
      while (lo < hi - 1) {
        const mid = (lo + hi) >> 1;
        if (frames[mid].tMs <= tMs) lo = mid;
        else hi = mid;
      }
      const a = frames[lo];
      const b = frames[hi];
      if (a.tMs === b.tMs || tMs === a.tMs) return a;
      const t = (tMs - a.tMs) / (b.tMs - a.tMs);
      return lerpFrame(a, b, t);
    },

    play({ speed: s, fromMs }) {
      if (intervalId || !segment) return;
      speed = s;
      currentMs = fromMs ?? currentMs;
      intervalId = setInterval(tickFrame, DEFAULT_SAMPLE_INTERVAL_MS);
    },

    pause() {
      if (intervalId) {
        clearInterval(intervalId);
        intervalId = null;
      }
    },

    setSpeed(s: number) {
      speed = s;
    },

    currentSegment() {
      return segment;
    },

    currentTimeMs() {
      return currentMs;
    },

    isPlaying() {
      return intervalId !== null;
    },

    destroy() {
      if (intervalId) {
        clearInterval(intervalId);
        intervalId = null;
      }
      segment = null;
      currentMs = 0;
      speed = 1;
    },
  };

  return handle;
}