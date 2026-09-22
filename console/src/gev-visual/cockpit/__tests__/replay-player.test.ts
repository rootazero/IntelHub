import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { mountCockpitReplayPlayer } from "../replay-player";
import type { ReplaySegment } from "../replay-types";

function makeSegment(durationMs: number, frameCount: number): ReplaySegment {
  const frames = [];
  for (let i = 0; i < frameCount; i++) {
    frames.push({
      tMs: (i * durationMs) / frameCount,
      frame: {
        heading: (i * 360) / frameCount,
        pitchRad: 0,
        bankRad: 0,
        altitudeFt: 1000 + i,
        speedKt: 100,
        vsiMps: 0,
        callsign: "TEST",
      },
    });
  }
  return {
    id: "test-seg",
    createdAt: new Date().toISOString(),
    durationMs,
    sampleIntervalMs: durationMs / frameCount,
    frames,
  };
}

describe("mountCockpitReplayPlayer", () => {
  beforeEach(() => {
    vi.useRealTimers();
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  test("loads a segment via load()", async () => {
    const seg = makeSegment(1000, 20);
    const player = mountCockpitReplayPlayer({});
    await player.load(seg);
    expect(player.currentSegment()?.id).toBe("test-seg");
  });

  test("seek(t) returns the frame at time t (with interpolation)", async () => {
    const seg = makeSegment(1000, 20);
    const player = mountCockpitReplayPlayer({});
    await player.load(seg);
    const f = player.seek(500); // halfway
    expect(f).not.toBeNull();
    expect(f!.frame.altitudeFt).toBeCloseTo(1010, 0);
  });

  test("seek(0) returns first frame; seek(durationMs) returns last frame", async () => {
    const seg = makeSegment(1000, 20);
    const player = mountCockpitReplayPlayer({});
    await player.load(seg);
    expect(player.seek(0)!.frame.altitudeFt).toBe(1000);
    expect(player.seek(1000)!.frame.altitudeFt).toBe(1020);
  });

  test("seek beyond end returns null", async () => {
    const seg = makeSegment(1000, 20);
    const player = mountCockpitReplayPlayer({});
    await player.load(seg);
    expect(player.seek(2000)).toBeNull();
    expect(player.seek(-1)).toBeNull();
  });

  test("play() drives onTick at ~20 Hz (50ms interval)", async () => {
    const seg = makeSegment(1000, 20);
    const ticks: number[] = [];
    const player = mountCockpitReplayPlayer({
      onTick: (f) => ticks.push(f.frame.altitudeFt),
    });
    await player.load(seg);
    player.play({ speed: 1 });
    await new Promise((r) => setTimeout(r, 250));
    player.pause();
    expect(ticks.length).toBeGreaterThanOrEqual(3);
    expect(ticks.length).toBeLessThanOrEqual(6);
  });

  test("pause() stops the play loop", async () => {
    const seg = makeSegment(1000, 20);
    let ticks = 0;
    const player = mountCockpitReplayPlayer({
      onTick: () => ticks++,
    });
    await player.load(seg);
    player.play({ speed: 1 });
    await new Promise((r) => setTimeout(r, 200));
    const before = ticks;
    player.pause();
    await new Promise((r) => setTimeout(r, 300));
    expect(ticks).toBe(before); // no more ticks after pause
  });

  test("play() auto-pauses when reaching end of segment", async () => {
    const seg = makeSegment(100, 5); // 100ms total
    const player = mountCockpitReplayPlayer({});
    await player.load(seg);
    player.play({ speed: 1 });
    await new Promise((r) => setTimeout(r, 200));
    expect(player.isPlaying()).toBe(false);
  });

  test("destroy() clears state and stops play", async () => {
    const seg = makeSegment(1000, 20);
    const player = mountCockpitReplayPlayer({});
    await player.load(seg);
    player.play({ speed: 1 });
    player.destroy();
    expect(player.currentSegment()).toBeNull();
    expect(player.isPlaying()).toBe(false);
  });

  test("setSpeed() updates playback speed mid-play", async () => {
    const seg = makeSegment(2000, 40);
    const player = mountCockpitReplayPlayer({});
    await player.load(seg);
    player.play({ speed: 1 });
    player.setSpeed(4);
    await new Promise((r) => setTimeout(r, 150));
    // At 4x speed: 150ms real = ~600ms playback, should be well past 200ms.
    expect(player.currentTimeMs()).toBeGreaterThanOrEqual(200);
    player.pause();
  });

  test("play() without loaded segment is a no-op", () => {
    const player = mountCockpitReplayPlayer({});
    player.play({ speed: 1 });
    expect(player.isPlaying()).toBe(false);
  });

  test("seek handles heading wrap (shortest angular path)", async () => {
    const seg: ReplaySegment = {
      id: "wrap",
      createdAt: new Date().toISOString(),
      durationMs: 1000,
      sampleIntervalMs: 100,
      frames: [
        {
          tMs: 0,
          frame: {
            heading: 350,
            pitchRad: 0,
            bankRad: 0,
            altitudeFt: 1000,
            speedKt: 100,
            vsiMps: 0,
            callsign: "X",
          },
        },
        {
          tMs: 1000,
          frame: {
            heading: 10, // wrapped from 350 → 10 (shortest is +20°, not +380°)
            pitchRad: 0,
            bankRad: 0,
            altitudeFt: 1000,
            speedKt: 100,
            vsiMps: 0,
            callsign: "X",
          },
        },
      ],
    };
    const player = mountCockpitReplayPlayer({});
    await player.load(seg);
    const mid = player.seek(500);
    // 350 + 20°/2 = 360° (which mod 360 = 0°), heading should be near 0
    expect(mid!.frame.heading).toBeCloseTo(0, 0);
    expect(mid!.frame.heading).not.toBeGreaterThan(180);
  });
});