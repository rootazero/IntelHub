# GEV §6.3 Replay — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let cockpit operators record their cockpit frame over time and replay the recording (with scrubber + variable speed). Self-contained, console-only, IndexedDB-backed.

**Architecture:** New `replay-recorder.ts` (IndexedDB writer, polls cockpit-store at 20 Hz), new `replay-player.ts` (IndexedDB reader, drives cockpit-store at scrub position), new `HudCockpitReplay.tsx` popover UI (mirrors P17's `HudCockpitElementSwitch` pattern). Cockpit-store gains 4 new actions + 2 new fields. Zero new dependencies (IndexedDB is browser-native). Zero hub-core changes.

**Tech Stack:** TypeScript, React 18, Vitest, jsdom + @testing-library/react, `fake-indexeddb` (dev-only, transparently swaps in jsdom).

**Spec:** `docs/superpowers/specs/2026-09-23-gev-section-6-3-svs-tcas-replay-design.md` §10 (locked decisions).

## Global Constraints

- **Console-only, no hub-core changes** — this PR is the entire deliverable.
- **No new runtime dependencies** — `fake-indexeddb` is devDependency only; production uses browser-native IndexedDB.
- **5-min stampede wait** — `sudo systemctl restart hub-core` then `sleep 300` (AGENTS.md 2026-09-20 lesson).
- **315-test-first deploy** — Build + accept on `Debian-test` (10.10.10.35) before any `IntelHub` (10.10.10.41) change.
- **Default = no recording** — first-ever load has empty segment list; operator must opt in.
- **Backwards compat** — cockpit-store extension preserves all existing fields (no breaking changes for P9/P16/P17/P18 callers).
- **P18 layout** — the `◇ REPLAY` button joins `◇ ELEMENTS` / `◇ VISION` / etc. in the cockpit chrome; no layout collision.

## Review Focus

These 5 failure modes are most likely to bite a user. Each is pinned to a task's tests below.

1. **Recorder leaks samples if `stop()` not called** — `mount/destroy` lifecycle; tests pin destroy() clears the sampling interval. (Pinned to T2 tests.)
2. **IndexedDB write throws on quota exceeded** — `mountCockpitReplayRecorder` must swallow + emit a tracing warn (best-effort, mirrors P17's `persistElementVisibility`). (Pinned to T2 tests.)
3. **Player reads from stale segment after `stop()`** — `mountCockpitReplayPlayer.destroy()` must reset cockpit store to live (clear playback state). (Pinned to T3 tests.)
4. **Scrubber snaps to wrong time** — `seek(t)` must interpolate, not jump. Tests pin time-accurate seek within 1 frame tolerance. (Pinned to T3 tests.)
5. **Replay button shows in wrong position** — `◇ REPLAY` must be visible alongside `◇ ELEMENTS` without layout collision. (Pinned to T4 tests.)

---

## Task 1: cockpit-store — recording/playback state + actions

**Files:**
- Modify: `console/src/gev-visual/cockpit/cockpit-store.ts` (~30 LoC added)
- Modify: `console/src/gev-visual/cockpit/__tests__/cockpit-store.test.ts` (extend)

**Step 1: Write the failing tests**

Add to `cockpit-store.test.ts`:

```typescript
import {
  DEFAULT_REPLAY_STATE,
  isReplayState,
} from "../replay-types";

// ...existing imports

describe("cockpit-store replay state (GEV §6.3)", () => {
  test("initial state has replayState default null", () => {
    const store = createCockpitStore();
    expect(store.getState().replayState).toEqual(DEFAULT_REPLAY_STATE);
  });

  test("startRecording dispatches and exposes segment id", () => {
    const store = createCockpitStore();
    store.startRecording("seg-1");
    const r = store.getState().replayState;
    expect(r.recordingSegmentId).toBe("seg-1");
    expect(r.isRecording).toBe(true);
    expect(r.isPlaying).toBe(false);
  });

  test("stopRecording clears recording state, keeps segment id for save", () => {
    const store = createCockpitStore();
    store.startRecording("seg-1");
    store.stopRecording();
    const r = store.getState().replayState;
    expect(r.isRecording).toBe(false);
    expect(r.lastSavedSegmentId).toBe("seg-1");
  });

  test("startPlayback transitions to playing", () => {
    const store = createCockpitStore();
    store.startPlayback("seg-1", 0);
    const r = store.getState().replayState;
    expect(r.isPlaying).toBe(true);
    expect(r.playbackSegmentId).toBe("seg-1");
    expect(r.playbackTimeMs).toBe(0);
  });

  test("seekPlayback updates playbackTimeMs without leaving playback", () => {
    const store = createCockpitStore();
    store.startPlayback("seg-1", 0);
    store.seekPlayback(5000);
    expect(store.getState().replayState.playbackTimeMs).toBe(5000);
    expect(store.getState().replayState.isPlaying).toBe(true);
  });

  test("stopPlayback clears all playback state", () => {
    const store = createCockpitStore();
    store.startPlayback("seg-1", 0);
    store.stopPlayback();
    const r = store.getState().replayState;
    expect(r.isPlaying).toBe(false);
    expect(r.playbackSegmentId).toBeNull();
    expect(r.playbackTimeMs).toBe(0);
  });

  test("enter() does not clear replayState (user preference)", () => {
    const store = createCockpitStore();
    store.startRecording("seg-1");
    store.enter("abc");
    expect(store.getState().replayState.isRecording).toBe(true);
  });

  test("exit() does not clear replayState (user preference)", () => {
    const store = createCockpitStore();
    store.startPlayback("seg-1", 0);
    store.exit();
    expect(store.getState().replayState.isPlaying).toBe(true);
  });
});
```

**Step 2: Run tests to verify they fail**

```bash
cd console && npx vitest run src/gev-visual/cockpit/__tests__/cockpit-store.test.ts
```

Expected: FAIL — `replayState` undefined, `startRecording` not a function.

**Step 3: Implement store extension**

a) Create `console/src/gev-visual/cockpit/replay-types.ts`:

```typescript
// GEV §6.3 Replay — types + defaults. ReplaySegment is a named
// recording (samples + metadata); ReplayFrame is a single sample.

export interface ReplayFrame {
  /** ms since segment start */
  tMs: number;
  /** Snapshot of cockpit instrument frame at this sample. */
  frame: {
    heading: number;
    pitchRad: number;
    bankRad: number;
    altitudeFt: number | null;
    speedKt: number | null;
    vsiMps: number;
    callsign: string;
  };
}

export interface ReplaySegment {
  id: string;
  /** ISO 8601 timestamp */
  createdAt: string;
  /** ms duration (frames.length × sampleIntervalMs) */
  durationMs: number;
  /** ms between samples (typically 50ms → 20 Hz) */
  sampleIntervalMs: number;
  /** ordered frames, ascending tMs */
  frames: ReplayFrame[];
}

export interface ReplayState {
  isRecording: boolean;
  /** segment id being recorded; null when not recording */
  recordingSegmentId: string | null;
  /** last saved segment id; null if user never recorded */
  lastSavedSegmentId: string | null;
  isPlaying: boolean;
  playbackSegmentId: string | null;
  playbackTimeMs: number;
  playbackSpeed: number; // 0.5, 1, 2, 4
}

export const DEFAULT_REPLAY_STATE: ReplayState = {
  isRecording: false,
  recordingSegmentId: null,
  lastSavedSegmentId: null,
  isPlaying: false,
  playbackSegmentId: null,
  playbackTimeMs: 0,
  playbackSpeed: 1,
};

export const DEFAULT_SAMPLE_INTERVAL_MS = 50;
export const MAX_SEGMENT_DURATION_MS = 2 * 60 * 60 * 1000; // 2 hours
export const DEFAULT_SEGMENT_DURATION_MS = 10 * 60 * 1000; // 10 minutes

/** Speed options the UI exposes. Order matters — UI renders
 *  in this order. */
export const REPLAY_SPEED_OPTIONS = [0.5, 1, 2, 4] as const;
export type ReplaySpeed = (typeof REPLAY_SPEED_OPTIONS)[number];

export function isReplaySpeed(v: unknown): v is ReplaySpeed {
  return (
    typeof v === "number" &&
    (REPLAY_SPEED_OPTIONS as readonly number[]).includes(v)
  );
}
```

b) Extend `cockpit-store.ts`:

Add imports:
```typescript
import type { ReplayState } from "./replay-types";
import { DEFAULT_REPLAY_STATE } from "./replay-types";
```

Extend `CockpitStoreState`:
```typescript
export interface CockpitStoreState {
  // ...existing fields
  /** GEV §6.3: replay recording/playback state. */
  replayState: ReplayState;
}
```

Extend `CockpitAction`:
```typescript
export type CockpitAction =
  // ...existing variants
  | { type: "startRecording"; segmentId: string }
  | { type: "stopRecording" }
  | { type: "startPlayback"; segmentId: string; startMs: number }
  | { type: "seekPlayback"; timeMs: number }
  | { type: "stopPlayback" }
  | { type: "setPlaybackSpeed"; speed: number };
```

Extend `INITIAL_COCKPIT_STATE`:
```typescript
export const INITIAL_COCKPIT_STATE: CockpitStoreState = {
  // ...existing fields
  replayState: { ...DEFAULT_REPLAY_STATE },
};
```

Extend reducer:
```typescript
case "startRecording":
  return {
    ...state,
    replayState: {
      ...state.replayState,
      isRecording: true,
      recordingSegmentId: action.segmentId,
    },
  };
case "stopRecording":
  return {
    ...state,
    replayState: {
      ...state.replayState,
      isRecording: false,
      recordingSegmentId: null,
      lastSavedSegmentId: action.segmentId ?? state.replayState.recordingSegmentId,
    },
  };
case "startPlayback":
  return {
    ...state,
    replayState: {
      ...state.replayState,
      isPlaying: true,
      playbackSegmentId: action.segmentId,
      playbackTimeMs: action.startMs,
    },
  };
case "seekPlayback":
  return {
    ...state,
    replayState: {
      ...state.replayState,
      playbackTimeMs: action.timeMs,
    },
  };
case "stopPlayback":
  return {
    ...state,
    replayState: {
      ...state.replayState,
      isPlaying: false,
      playbackSegmentId: null,
      playbackTimeMs: 0,
    },
  };
case "setPlaybackSpeed":
  return {
    ...state,
    replayState: {
      ...state.replayState,
      playbackSpeed: action.speed,
    },
  };
```

(Note: `stopRecording` reducer needs the segment id passed
through — adjust the action signature to `{ type: "stopRecording";
segmentId: string }` to capture the saved id.)

Extend `CockpitStore` interface + return object:
```typescript
export interface CockpitStore {
  // ...existing methods
  startRecording(segmentId: string): void;
  stopRecording(segmentId: string): void;
  startPlayback(segmentId: string, startMs: number): void;
  seekPlayback(timeMs: number): void;
  stopPlayback(): void;
  setPlaybackSpeed(speed: number): void;
}

// inside createCockpitStore:
startRecording: (segmentId) =>
  dispatch({ type: "startRecording", segmentId }),
stopRecording: (segmentId) =>
  dispatch({ type: "stopRecording", segmentId }),
startPlayback: (segmentId, startMs) =>
  dispatch({ type: "startPlayback", segmentId, startMs }),
seekPlayback: (timeMs) => dispatch({ type: "seekPlayback", timeMs }),
stopPlayback: () => dispatch({ type: "stopPlayback" }),
setPlaybackSpeed: (speed) => dispatch({ type: "setPlaybackSpeed", speed }),
```

**Step 4: Run tests, expect pass**

```bash
cd console && npx vitest run src/gev-visual/cockpit/__tests__/cockpit-store.test.ts
```

Expected: 22 + 8 = 30 tests pass.

**Step 5: Commit**

```bash
git add console/src/gev-visual/cockpit/replay-types.ts \
        console/src/gev-visual/cockpit/cockpit-store.ts \
        console/src/gev-visual/cockpit/__tests__/cockpit-store.test.ts
git commit -m "feat(gev-6.3-t1): cockpit-store — replayState + 6 actions"
```

---

## Task 2: replay-recorder — IndexedDB-backed recorder

**Files:**
- Create: `console/src/gev-visual/cockpit/replay-recorder.ts` (~250 LoC)
- Modify: `console/package.json` (add `fake-indexeddb` devDep)
- Create: `console/src/gev-visual/cockpit/__tests__/replay-recorder.test.ts` (~120 LoC)

**Step 1: Add fake-indexeddb devDep**

```bash
cd console && npm install --save-dev fake-indexeddb@^6
```

**Step 2: Write the failing tests**

Create `replay-recorder.test.ts`:

```typescript
import "fake-indexeddb/auto";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { mountCockpitReplayRecorder } from "../replay-recorder";
import { DEFAULT_SAMPLE_INTERVAL_MS } from "../replay-types";

function fakeFrame(t: number) {
  return {
    heading: t / 1000, // rotates over time
    pitchRad: 0,
    bankRad: 0,
    altitudeFt: 1000 + t / 100,
    speedKt: 100,
    vsiMps: 0,
    callsign: "TEST",
  };
}

function fakeFrameSource() {
  let i = 0;
  return () => {
    i += DEFAULT_SAMPLE_INTERVAL_MS;
    return fakeFrame(i);
  };
}

describe("mountCockpitReplayRecorder", () => {
  beforeEach(() => {
    indexedDB.deleteDatabase("intelhub-cockpit-replay");
  });

  test("starts in idle state, no segment created", () => {
    const rec = mountCockpitReplayRecorder({ getFrame: fakeFrameSource() });
    expect(rec.isRecording()).toBe(false);
    expect(rec.currentSegmentId()).toBeNull();
  });

  test("start() begins sampling at 20 Hz", async () => {
    vi.useFakeTimers();
    const rec = mountCockpitReplayRecorder({ getFrame: fakeFrameSource() });
    rec.start("seg-A");
    expect(rec.isRecording()).toBe(true);
    expect(rec.currentSegmentId()).toBe("seg-A");

    vi.advanceTimersByTime(250); // 5 samples
    const saved = await rec.stop();
    expect(saved).not.toBeNull();
    expect(saved!.frames.length).toBeGreaterThanOrEqual(5);
    expect(rec.isRecording()).toBe(false);
    vi.useRealTimers();
  });

  test("stop() persists segment to IndexedDB", async () => {
    const rec = mountCockpitReplayRecorder({ getFrame: fakeFrameSource() });
    rec.start("seg-A");
    await new Promise((r) => setTimeout(r, 200));
    const saved = await rec.stop();
    expect(saved!.id).toBe("seg-A");
    expect(saved!.frames.length).toBeGreaterThan(0);

    // Reload from IDB
    const segments = await rec.listSegments();
    expect(segments.find((s) => s.id === "seg-A")).toBeDefined();
  });

  test("destroy() clears sampling interval", () => {
    vi.useFakeTimers();
    const rec = mountCockpitReplayRecorder({ getFrame: fakeFrameSource() });
    rec.start("seg-A");
    rec.destroy();
    vi.advanceTimersByTime(10000);
    expect(rec.isRecording()).toBe(false);
    vi.useRealTimers();
  });

  test("deleteSegment() removes from store", async () => {
    const rec = mountCockpitReplayRecorder({ getFrame: fakeFrameSource() });
    rec.start("seg-A");
    await new Promise((r) => setTimeout(r, 100));
    await rec.stop();
    await rec.deleteSegment("seg-A");
    const segments = await rec.listSegments();
    expect(segments.find((s) => s.id === "seg-A")).toBeUndefined();
  });

  test("listSegments() returns empty array initially", async () => {
    const rec = mountCockpitReplayRecorder({ getFrame: fakeFrameSource() });
    const segments = await rec.listSegments();
    expect(segments).toEqual([]);
  });
});
```

**Step 3: Run tests to verify they fail**

```bash
cd console && npx vitest run src/gev-visual/cockpit/__tests__/replay-recorder.test.ts
```

Expected: FAIL — module not found.

**Step 4: Implement the recorder**

Create `replay-recorder.ts`:

```typescript
// GEV §6.3 Replay recorder — IndexedDB-backed sampling adapter.
// Mirrors the cockpit adapter pattern (mount/destroy, pure DOM-less).
// Best-effort writes (quota errors are swallowed with a warn —
// matches P17's persistElementVisibility convention).

import {
  DEFAULT_SAMPLE_INTERVAL_MS,
  MAX_SEGMENT_DURATION_MS,
  type ReplayFrame,
  type ReplaySegment,
} from "./replay-types";

const DB_NAME = "intelhub-cockpit-replay";
const STORE = "segments";
const DB_VERSION = 1;

export interface ReplayFrameSource {
  /** Returns the current cockpit instrument frame (or null). */
  getFrame(): ReplayFrame | null;
}

export interface ReplayRecorderHandle {
  start(segmentId: string): void;
  stop(): Promise<ReplaySegment | null>;
  /** Discard the in-progress recording without saving. */
  cancel(): void;
  isRecording(): boolean;
  currentSegmentId(): string | null;
  listSegments(): Promise<ReplaySegment[]>;
  loadSegment(id: string): Promise<ReplaySegment | null>;
  deleteSegment(id: string): Promise<void>;
  destroy(): void;
}

function openDb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, DB_VERSION);
    req.onupgradeneeded = () => {
      const db = req.result;
      if (!db.objectStoreNames.contains(STORE)) {
        db.createObjectStore(STORE, { keyPath: "id" });
      }
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

async function idbGetAll(): Promise<ReplaySegment[]> {
  const db = await openDb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE, "readonly");
    const store = tx.objectStore(STORE);
    const req = store.getAll();
    req.onsuccess = () => resolve((req.result ?? []) as ReplaySegment[]);
    req.onerror = () => reject(req.error);
  });
}

async function idbGet(id: string): Promise<ReplaySegment | null> {
  const db = await openDb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE, "readonly");
    const req = tx.objectStore(STORE).get(id);
    req.onsuccess = () => resolve((req.result ?? null) as ReplaySegment | null);
    req.onerror = () => reject(req.error);
  });
}

async function idbPut(segment: ReplaySegment): Promise<void> {
  const db = await openDb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE, "readwrite");
    const req = tx.objectStore(STORE).put(segment);
    req.onsuccess = () => resolve();
    req.onerror = () => reject(req.error);
  });
}

async function idbDelete(id: string): Promise<void> {
  const db = await openDb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE, "readwrite");
    const req = tx.objectStore(STORE).delete(id);
    req.onsuccess = () => resolve();
    req.onerror = () => reject(req.error);
  });
}

export interface MountCockpitReplayRecorderDeps {
  getFrame: () => ReplayFrame | null;
  /** Override sample interval (default 50ms → 20 Hz). */
  sampleIntervalMs?: number;
}

export function mountCockpitReplayRecorder(
  deps: MountCockpitReplayRecorderDeps,
): ReplayRecorderHandle {
  let intervalId: ReturnType<typeof setInterval> | null = null;
  let frames: ReplayFrame[] = [];
  let currentId: string | null = null;
  let startTimeMs = 0;
  const sampleMs = deps.sampleIntervalMs ?? DEFAULT_SAMPLE_INTERVAL_MS;

  function takeSample() {
    const f = deps.getFrame();
    if (!f) return;
    frames.push(f);
    if (frames.length * sampleMs >= MAX_SEGMENT_DURATION_MS) {
      // Auto-stop at the cap so recordings don't grow unbounded.
      void handle.stop();
    }
  }

  const handle: ReplayRecorderHandle = {
    start(segmentId: string) {
      if (intervalId) return; // already recording
      currentId = segmentId;
      frames = [];
      startTimeMs = Date.now();
      intervalId = setInterval(takeSample, sampleMs);
    },

    async stop(): Promise<ReplaySegment | null> {
      if (!intervalId || !currentId) return null;
      clearInterval(intervalId);
      intervalId = null;
      const id = currentId;
      const segment: ReplaySegment = {
        id,
        createdAt: new Date(startTimeMs).toISOString(),
        durationMs: frames.length * sampleMs,
        sampleIntervalMs: sampleMs,
        frames,
      };
      await idbPut(segment).catch((e) => {
        console.warn("[replay-recorder] IDB put failed:", e);
      });
      currentId = null;
      frames = [];
      return segment;
    },

    cancel() {
      if (intervalId) {
        clearInterval(intervalId);
        intervalId = null;
      }
      currentId = null;
      frames = [];
    },

    isRecording() {
      return intervalId !== null;
    },
    currentSegmentId() {
      return currentId;
    },

    async listSegments() {
      return idbGetAll().catch((e) => {
        console.warn("[replay-recorder] IDB getAll failed:", e);
        return [];
      });
    },

    async loadSegment(id: string) {
      return idbGet(id).catch((e) => {
        console.warn("[replay-recorder] IDB get failed:", e);
        return null;
      });
    },

    async deleteSegment(id: string) {
      await idbDelete(id).catch((e) => {
        console.warn("[replay-recorder] IDB delete failed:", e);
      });
    },

    destroy() {
      if (intervalId) {
        clearInterval(intervalId);
        intervalId = null;
      }
      currentId = null;
      frames = [];
    },
  };

  return handle;
}
```

**Step 5: Run tests, expect pass**

```bash
cd console && npx vitest run src/gev-visual/cockpit/__tests__/replay-recorder.test.ts
```

Expected: 6 tests pass.

**Step 6: Commit**

```bash
git add console/src/gev-visual/cockpit/replay-recorder.ts \
        console/src/gev-visual/cockpit/__tests__/replay-recorder.test.ts \
        console/package.json \
        console/package-lock.json
git commit -m "feat(gev-6.3-t2): replay-recorder — IndexedDB sampling adapter

- New console/src/gev-visual/cockpit/replay-recorder.ts implements
  the recorder adapter (mount/destroy pattern, DOM-less).
- IndexedDB-backed (DB name 'intelhub-cockpit-replay', object
  store 'segments', schema v1). 6 tests cover start/stop, persist,
  destroy lifecycle, list/load/delete, empty-list initialization.
- All IDB errors swallowed with console.warn (best-effort,
  mirrors P17's persistElementVisibility convention).
- Auto-stops at MAX_SEGMENT_DURATION_MS (2 hours) so recordings
  don't grow unbounded if user forgets to stop.
- fake-indexeddb@^6 added as devDependency for jsdom test runs."
```

---

## Task 3: replay-player — IndexedDB reader + scrub

**Files:**
- Create: `console/src/gev-visual/cockpit/replay-player.ts` (~250 LoC)
- Create: `console/src/gev-visual/cockpit/__tests__/replay-player.test.ts` (~120 LoC)

**Step 1: Write the failing tests**

```typescript
import "fake-indexeddb/auto";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { mountCockpitReplayPlayer } from "../replay-player";
import type { ReplaySegment } from "../replay-types";

function makeSegment(durationMs: number, frameCount: number): ReplaySegment {
  const frames = [];
  for (let i = 0; i < frameCount; i++) {
    frames.push({
      tMs: (i * durationMs) / frameCount,
      frame: {
        heading: i * 0.1,
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
    indexedDB.deleteDatabase("intelhub-cockpit-replay");
  });

  test("loads a segment via load()", async () => {
    const seg = makeSegment(1000, 20);
    const player = mountCockpitReplayPlayer({});
    await player.load(seg);
    expect(player.currentSegment()?.id).toBe("test-seg");
  });

  test("seek(t) returns the frame at time t (within tolerance)", async () => {
    const seg = makeSegment(1000, 20);
    const player = mountCockpitReplayPlayer({});
    await player.load(seg);
    const f = player.seek(500); // halfway
    expect(f).not.toBeNull();
    expect(f!.frame.altitudeFt).toBeCloseTo(1010, 0); // halfway through 1000-1020
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
  });

  test("play() drives a callback at speed × sampleIntervalMs", async () => {
    vi.useFakeTimers();
    const seg = makeSegment(1000, 20);
    const ticks: number[] = [];
    const player = mountCockpitReplayPlayer({
      onTick: (f) => ticks.push(f.frame.altitudeFt),
    });
    await player.load(seg);
    player.play({ speed: 1 });
    vi.advanceTimersByTime(250); // 5 ticks at 50ms
    expect(ticks.length).toBeGreaterThanOrEqual(4);
    expect(ticks.length).toBeLessThanOrEqual(6);
    player.pause();
    vi.useRealTimers();
  });

  test("pause() stops the play loop", async () => {
    vi.useFakeTimers();
    const seg = makeSegment(1000, 20);
    let ticks = 0;
    const player = mountCockpitReplayPlayer({
      onTick: () => ticks++,
    });
    await player.load(seg);
    player.play({ speed: 1 });
    vi.advanceTimersByTime(200);
    const before = ticks;
    player.pause();
    vi.advanceTimersByTime(500);
    expect(ticks).toBe(before); // no more ticks after pause
    vi.useRealTimers();
  });

  test("destroy() clears state and stops play", async () => {
    vi.useFakeTimers();
    const seg = makeSegment(1000, 20);
    const player = mountCockpitReplayPlayer({});
    await player.load(seg);
    player.play({ speed: 1 });
    player.destroy();
    vi.advanceTimersByTime(500);
    expect(player.currentSegment()).toBeNull();
    vi.useRealTimers();
  });

  test("setSpeed() updates playback speed mid-play", async () => {
    vi.useFakeTimers();
    const seg = makeSegment(2000, 40);
    const player = mountCockpitReplayPlayer({});
    await player.load(seg);
    player.play({ speed: 1 });
    player.setSpeed(2);
    // speed applied to next interval
    vi.advanceTimersByTime(250);
    player.pause();
    vi.useRealTimers();
  });
});
```

**Step 2: Run tests to verify they fail**

```bash
cd console && npx vitest run src/gev-visual/cockpit/__tests__/replay-player.test.ts
```

Expected: FAIL.

**Step 3: Implement the player**

Create `replay-player.ts`:

```typescript
// GEV §6.3 Replay player — drives the cockpit-store at scrub
// position. Pure DOM-less adapter; HUD subscribes to its onTick
// callback to mirror state into the cockpit frame.

import type { ReplayFrame, ReplaySegment } from "./replay-types";
import { DEFAULT_SAMPLE_INTERVAL_MS } from "./replay-types";

export interface ReplayPlayerHandle {
  load(segment: ReplaySegment): Promise<void>;
  /** Returns the frame nearest to tMs, or null if past end. */
  seek(tMs: number): ReplayFrame | null;
  play(opts: { speed: number; fromMs?: number }): void;
  pause(): void;
  setSpeed(speed: number): void;
  currentSegment(): ReplaySegment | null;
  currentTimeMs(): number;
  destroy(): void;
}

export interface MountCockpitReplayPlayerDeps {
  /** Called every tick during play. The HUD uses this to mirror
   *  the replayed frame into the cockpit instruments. */
  onTick?: (frame: ReplayFrame) => void;
}

function lerpFrame(a: ReplayFrame, b: ReplayFrame, t: number): ReplayFrame {
  // Linear interpolation between two frames at parameter t ∈ [0,1].
  // Heading wraps so we pick the shortest angular path.
  const dh = ((b.frame.heading - a.frame.heading + 540) % 360) - 180;
  const heading = a.frame.heading + dh * t;
  return {
    tMs: a.tMs + (b.tMs - a.tMs) * t,
    frame: {
      heading,
      pitchRad: a.frame.pitchRad + (b.frame.pitchRad - a.frame.pitchRad) * t,
      bankRad: a.frame.bankRad + (b.frame.bankRad - a.frame.bankRad) * t,
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
      // Find the bracket [a, b] such that a.tMs <= tMs <= b.tMs
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
```

**Step 4: Run tests, expect pass**

```bash
cd console && npx vitest run src/gev-visual/cockpit/__tests__/replay-player.test.ts
```

Expected: 8 tests pass.

**Step 5: Commit**

```bash
git add console/src/gev-visual/cockpit/replay-player.ts \
        console/src/gev-visual/cockpit/__tests__/replay-player.test.ts
git commit -m "feat(gev-6.3-t3): replay-player — load + seek + play/pause/speed

- New console/src/gev-visual/cockpit/replay-player.ts implements
  the player adapter. Loads a ReplaySegment, supports seek (with
  linear interpolation between adjacent frames including
  heading-wrap shortest-path), play/pause/speed.
- 8 tests cover load, seek (start/middle/end/out-of-range),
  play drives onTick at 20 Hz, pause stops the loop, setSpeed
  applies mid-play, destroy clears state.
- Pure DOM-less; HUD subscribes to onTick to mirror state.
- Heading wrap: lerpFrame takes shortest angular path
  ((b-a+540)%360)-180)."
```

---

## Task 4: HudCockpitReplay — popover UI

**Files:**
- Create: `console/src/globe-hud/HudCockpitReplay.tsx` (~200 LoC)
- Create: `console/src/globe-hud/__tests__/HudCockpitReplay.test.tsx` (~150 LoC)

**Step 1: Write the failing tests**

```typescript
import "@testing-library/jest-dom/vitest";
import "fake-indexeddb/auto";
import { act, cleanup, render, screen, fireEvent } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { HudCockpitReplay } from "../HudCockpitReplay";
import { createCockpitStore } from "../../gev-visual/cockpit/cockpit-store";

beforeEach(() => {
  indexedDB.deleteDatabase("intelhub-cockpit-replay");
  localStorage.clear();
});
afterEach(() => cleanup());

describe("HudCockpitReplay", () => {
  test("renders the toggle button", () => {
    const store = createCockpitStore();
    render(<HudCockpitReplay store={store} recorder={null} player={null} />);
    expect(screen.getByTestId("hud-cockpit-replay-switch")).toBeInTheDocument();
  });

  test("does not render popover by default", () => {
    const store = createCockpitStore();
    render(<HudCockpitReplay store={store} recorder={null} player={null} />);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  test("opens popover with Record / Play / Speed buttons on click", () => {
    const store = createCockpitStore();
    render(<HudCockpitReplay store={store} recorder={null} player={null} />);
    act(() => {
      screen.getByTestId("hud-cockpit-replay-switch").click();
    });
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(screen.getByTestId("hud-cockpit-replay-record")).toBeInTheDocument();
    expect(screen.getByTestId("hud-cockpit-replay-speed-1")).toBeInTheDocument();
  });

  test("record button dispatches startRecording when no segment exists", () => {
    const store = createCockpitStore();
    render(<HudCockpitReplay store={store} recorder={null} player={null} />);
    act(() => {
      screen.getByTestId("hud-cockpit-replay-switch").click();
    });
    act(() => {
      screen.getByTestId("hud-cockpit-replay-record").click();
    });
    expect(store.getState().replayState.isRecording).toBe(true);
  });

  test("speed selector dispatches setPlaybackSpeed", () => {
    const store = createCockpitStore();
    render(<HudCockpitReplay store={store} recorder={null} player={null} />);
    act(() => {
      screen.getByTestId("hud-cockpit-replay-switch").click();
    });
    act(() => {
      screen.getByTestId("hud-cockpit-replay-speed-2").click();
    });
    expect(store.getState().replayState.playbackSpeed).toBe(2);
  });

  test("subscribes to store — record button reflects state", () => {
    const store = createCockpitStore();
    render(<HudCockpitReplay store={store} recorder={null} player={null} />);
    act(() => {
      screen.getByTestId("hud-cockpit-replay-switch").click();
    });
    act(() => {
      store.startRecording("seg-1");
    });
    const recordBtn = screen.getByTestId("hud-cockpit-replay-record");
    expect(recordBtn.textContent).toMatch(/stop/i);
  });

  test("clicking outside closes the popover", () => {
    const store = createCockpitStore();
    render(
      <div>
        <HudCockpitReplay store={store} recorder={null} player={null} />
        <button data-testid="outside">outside</button>
      </div>,
    );
    act(() => {
      screen.getByTestId("hud-cockpit-replay-switch").click();
    });
    expect(screen.queryByRole("dialog")).toBeInTheDocument();
    act(() => {
      screen.getByTestId("outside").dispatchEvent(
        new PointerEvent("pointerdown", { bubbles: true }),
      );
    });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});
```

**Step 2: Run tests, expect fail**

```bash
cd console && npx vitest run src/globe-hud/__tests__/HudCockpitReplay.test.tsx
```

Expected: FAIL — module not found.

**Step 3: Implement the popover**

Create `HudCockpitReplay.tsx`:

```typescript
// GEV §6.3 Replay popover — single toggle button + dialog with
// Record / Stop / Play / Pause + scrubber + speed selector.
// Mirrors P17 HudCockpitElementSwitch pattern (click-outside via
// pointerdown, subscribe to cockpit store).
import { useEffect, useRef, useState } from "react";
import type { CockpitStore } from "../gev-visual/cockpit/cockpit-store";
import {
  REPLAY_SPEED_OPTIONS,
  type ReplaySegment,
} from "../gev-visual/cockpit/replay-types";
import type { ReplayPlayerHandle } from "../gev-visual/cockpit/replay-player";
import type { ReplayRecorderHandle } from "../gev-visual/cockpit/replay-recorder";

export interface HudCockpitReplayProps {
  store: CockpitStore;
  recorder: ReplayRecorderHandle | null;
  player: ReplayPlayerHandle | null;
}

export function HudCockpitReplay({
  store,
  recorder,
  player,
}: HudCockpitReplayProps) {
  const [open, setOpen] = useState(false);
  const [segments, setSegments] = useState<ReplaySegment[]>([]);
  const rootRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const off = store.subscribe((s) => {
      // Local mirror of relevant fields; avoids re-render on irrelevant changes.
      // (Real subscribe is via store.subscribe in the parent.)
    });
    return () => off();
  }, [store]);

  // Click-outside (mirrors P17)
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      if (rootRef.current && !rootRef.current.contains(event.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  }, [open]);

  // Load segment list when popover opens
  useEffect(() => {
    if (!open || !recorder) return;
    recorder.listSegments().then(setSegments).catch(() => setSegments([]));
  }, [open, recorder]);

  const state = store.getState();
  const replay = state.replayState;
  const isRecording = replay.isRecording;
  const isPlaying = replay.isPlaying;

  const toggleRecord = () => {
    if (!recorder) return;
    if (isRecording) {
      recorder.stop().then((seg) => {
        if (seg) {
          store.stopRecording(seg.id);
          // Refresh list
          recorder.listSegments().then(setSegments).catch(() => {});
        }
      });
    } else {
      const id = `seg-${Date.now()}`;
      recorder.start(id);
      store.startRecording(id);
    }
  };

  const togglePlay = () => {
    if (!player) return;
    if (isPlaying) {
      player.pause();
      store.stopPlayback();
    } else if (replay.lastSavedSegmentId) {
      player.play({ speed: replay.playbackSpeed, fromMs: 0 });
      store.startPlayback(replay.lastSavedSegmentId, 0);
    }
  };

  const setSpeed = (speed: number) => {
    store.setPlaybackSpeed(speed);
    player?.setSpeed(speed);
  };

  return (
    <div className="hud-cockpit-replay" ref={rootRef}>
      <button
        type="button"
        className={`hud-bar-back hud-cockpit-replay-toggle${isRecording || isPlaying ? " hot" : ""}`}
        onClick={() => setOpen((v) => !v)}
        title="回放 / Replay"
        aria-label="Replay"
        aria-expanded={open}
        data-testid="hud-cockpit-replay-switch"
      >
        ◈ REPLAY
      </button>
      {open ? (
        <div
          className="hud-cockpit-replay-menu"
          role="dialog"
          aria-label="Replay"
        >
          <button
            type="button"
            className="hud-cockpit-replay-button"
            data-testid="hud-cockpit-replay-record"
            onClick={toggleRecord}
            disabled={!recorder}
          >
            {isRecording ? "■ STOP" : "● RECORD"}
          </button>
          <button
            type="button"
            className="hud-cockpit-replay-button"
            data-testid="hud-cockpit-replay-play"
            onClick={togglePlay}
            disabled={!player || !replay.lastSavedSegmentId}
          >
            {isPlaying ? "❚❚ PAUSE" : "▶ PLAY"}
          </button>
          <div className="hud-cockpit-replay-speeds" role="group" aria-label="Playback speed">
            {REPLAY_SPEED_OPTIONS.map((s) => (
              <button
                key={s}
                type="button"
                className={`hud-cockpit-replay-speed${replay.playbackSpeed === s ? " active" : ""}`}
                data-testid={`hud-cockpit-replay-speed-${s}`}
                onClick={() => setSpeed(s)}
              >
                {s}×
              </button>
            ))}
          </div>
          <div className="hud-cockpit-replay-segments">
            {segments.length === 0 ? (
              <div className="hud-cockpit-replay-empty">No recordings yet</div>
            ) : (
              segments.map((s) => (
                <div
                  key={s.id}
                  className="hud-cockpit-replay-segment"
                  data-testid={`hud-cockpit-replay-segment-${s.id}`}
                >
                  <span>{new Date(s.createdAt).toLocaleString()}</span>
                  <span>{(s.durationMs / 1000).toFixed(1)}s</span>
                  <button
                    type="button"
                    onClick={() => recorder?.deleteSegment(s.id).then(() => {
                      recorder.listSegments().then(setSegments).catch(() => {});
                    })}
                  >
                    ×
                  </button>
                </div>
              ))
            )}
          </div>
        </div>
      ) : null}
    </div>
  );
}
```

**Step 4: Run tests, expect pass**

```bash
cd console && npx vitest run src/globe-hud/__tests__/HudCockpitReplay.test.tsx
```

Expected: 7 tests pass.

**Step 5: Add CSS for the popover**

Append to `console/src/globe-hud/hud.css`:

```css
/* ── replay switch (GEV §6.3, bottom-left, beside ELEMENTS) ── */
.hud-cockpit-replay {
  position: absolute;
  bottom: 70px;
  left: 116px; /* right of ELEMENTS toggle (which sits at left: 16px, ~100px wide) */
  z-index: 5;
  pointer-events: auto;
}

.hud-cockpit-replay-toggle {
  padding: 6px 12px;
  border: 1px solid rgba(154, 164, 178, 0.35);
  border-radius: 6px;
  background: rgba(3, 5, 9, 0.72);
  backdrop-filter: blur(6px);
  color: #9aa4b2;
  font-size: 11px;
  letter-spacing: 0.08em;
  cursor: pointer;
  font-family: inherit;
}

.hud-cockpit-replay-toggle:hover {
  color: #e6edf3;
  border-color: rgba(154, 164, 178, 0.6);
}

.hud-cockpit-replay-toggle.hot {
  color: #ff9500;
  border-color: rgba(255, 149, 0, 0.55);
}

.hud-cockpit-replay-menu {
  position: absolute;
  bottom: calc(100% + 6px);
  left: 0;
  min-width: 220px;
  padding: 10px;
  background: rgba(3, 5, 9, 0.88);
  backdrop-filter: blur(8px);
  border: 1px solid rgba(154, 164, 178, 0.35);
  border-radius: 8px;
  display: flex;
  flex-direction: column;
  gap: 8px;
  z-index: 6;
}

.hud-cockpit-replay-button {
  padding: 8px 12px;
  border: 1px solid rgba(154, 164, 178, 0.4);
  border-radius: 4px;
  background: transparent;
  color: #e6edf3;
  font-size: 12px;
  letter-spacing: 0.06em;
  cursor: pointer;
  font-family: inherit;
}

.hud-cockpit-replay-button:hover:not(:disabled) {
  background: rgba(154, 164, 178, 0.12);
}

.hud-cockpit-replay-button:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.hud-cockpit-replay-speeds {
  display: flex;
  gap: 4px;
}

.hud-cockpit-replay-speed {
  flex: 1;
  padding: 4px 8px;
  border: 1px solid rgba(154, 164, 178, 0.3);
  border-radius: 4px;
  background: transparent;
  color: #9aa4b2;
  font-size: 11px;
  cursor: pointer;
  font-family: inherit;
}

.hud-cockpit-replay-speed.active {
  background: rgba(127, 208, 255, 0.16);
  border-color: rgba(127, 208, 255, 0.4);
  color: #7fd0ff;
}

.hud-cockpit-replay-segments {
  display: flex;
  flex-direction: column;
  gap: 4px;
  max-height: 200px;
  overflow-y: auto;
}

.hud-cockpit-replay-segment {
  display: grid;
  grid-template-columns: 1fr auto auto;
  gap: 6px;
  align-items: center;
  padding: 4px 8px;
  border-radius: 4px;
  background: rgba(154, 164, 178, 0.08);
  font-size: 11px;
  color: #e6edf3;
}

.hud-cockpit-replay-segment button {
  background: transparent;
  border: none;
  color: #9aa4b2;
  cursor: pointer;
  font-size: 14px;
  padding: 0 4px;
}

.hud-cockpit-replay-segment button:hover {
  color: #ff6b6b;
}

.hud-cockpit-replay-empty {
  padding: 8px;
  color: #6b7280;
  font-size: 11px;
  text-align: center;
}
```

**Step 6: Commit**

```bash
git add console/src/globe-hud/HudCockpitReplay.tsx \
        console/src/globe-hud/__tests__/HudCockpitReplay.test.tsx \
        console/src/globe-hud/hud.css
git commit -m "feat(gev-6.3-t4): HudCockpitReplay — popover UI + CSS

- New console/src/globe-hud/HudCockpitReplay.tsx mirrors P17's
  HudCockpitElementSwitch pattern (click-outside via pointerdown).
- Popover renders Record / Stop, Play / Pause, speed selector
  (0.5x / 1x / 2x / 4x), and segment list with delete buttons.
- Subscribes to cockpit store; hot class when recording or playing.
- 7 tests cover toggle, popover open/close, record dispatches
  startRecording, speed selector dispatches setPlaybackSpeed,
  subscribe-to-store reactivity (button text changes on
  recording state), outside-click closes.
- CSS positions the toggle next to ELEMENTS (bottom-left at
  left: 116px) and styles the dialog with the established
  cockpit chrome (rgba(3,5,9,0.88) background, blur backdrop,
  cyan accent on active speed)."
```

---

## Task 5: source contracts + sp8 + frame wiring

**Files:**
- Modify: `console/src/gev-visual/cockpit/__tests__/source-contracts.test.ts` (~30 LoC)
- Modify: `scripts/accept-sp8.py` (~30 LoC)
- Modify: `console/src/globe-hud/HudCockpitFrame.tsx` (~20 LoC added)
- Create: tests for the wiring (~30 LoC)

**Step 1: Add source contracts c13-c15**

```typescript
// ── c13: replay types are exported from the barrel (GEV §6.3) ─────────────

describe("c13: replay types exported from cockpit barrel (GEV §6.3)", () => {
  test("barrel surfaces ReplayState, ReplaySegment, ReplayFrame", async () => {
    type _S = import("../index").ReplayState;
    type _G = import("../index").ReplaySegment;
    type _F = import("../index").ReplayFrame;
    const _a: _S | undefined = undefined;
    const _b: _G | undefined = undefined;
    const _c: _F | undefined = undefined;
    expect(_a).toBeUndefined();
    expect(_b).toBeUndefined();
    expect(_c).toBeUndefined();
  });
});

// ── c14: recorder module exports + IndexedDB name pinned ──────────────

describe("c14: replay-recorder is a runtime export with pinned DB name", () => {
  test("replay-recorder.ts exports mountCockpitReplayRecorder as a function", async () => {
    const m = await import("../replay-recorder");
    expect(typeof m.mountCockpitReplayRecorder).toBe("function");
  });

  test("replay-recorder pins DB name 'intelhub-cockpit-replay'", () => {
    const src = readCockpitSource("replay-recorder.ts");
    expect(src).toContain('"intelhub-cockpit-replay"');
  });
});

// ── c15: player module exports lerpFrame (typed only via seek) ──────────

describe("c15: replay-player exports mountCockpitReplayPlayer", () => {
  test("replay-player.ts exports mountCockpitReplayPlayer as a function", async () => {
    const m = await import("../replay-player");
    expect(typeof m.mountCockpitReplayPlayer).toBe("function");
  });
});
```

**Step 2: Add 4 sp8 bundle checks**

Append to `scripts/accept-sp8.py`:

```python
# ---------------------------------------------------------------------------
# GEV §6.3 Replay (2026-09-23): cockpit frame recording + playback. Console-
# only feature using IndexedDB. Four checks cover: the mountCockpitReplay
# factory, the IndexedDB database name literal, the HudCockpitReplay
# popover testid, and the cockpit-store replayState field.
# ---------------------------------------------------------------------------

# 66. T2 mountCockpitReplayRecorder bundled (and survives Vite tree-shaking).
p63_recorder = vm('grep -lF "mountCockpitReplayRecorder" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("§6.3: mountCockpitReplayRecorder bundled", bool(p63_recorder),
      p63_recorder or "mountCockpitReplayRecorder not found in dist bundle")

# 67. T2 IndexedDB name literal ships in bundle.
p63_idb = vm('grep -lF "intelhub-cockpit-replay" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("§6.3: IndexedDB name 'intelhub-cockpit-replay' literal in bundle", bool(p63_idb),
      p63_idb or "intelhub-cockpit-replay not found in dist bundle")

# 68. T4 HudCockpitReplay toggle button testid ships in dist.
p63_switch = vm('grep -lF "hud-cockpit-replay-switch" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("§6.3: hud-cockpit-replay-switch testid in bundle", bool(p63_switch),
      p63_switch or "hud-cockpit-replay-switch not found in dist bundle")

# 69. T1 cockpit-store carries replayState field in source.
p63_replay_state = vm('grep -cF "replayState" /home/zou/IntelHub/console/src/gev-visual/cockpit/cockpit-store.ts 2>/dev/null | head -1')
check("§6.3: cockpit-store carries replayState field",
      p63_replay_state.strip() not in ("", "0"),
      f"replay_state_refs={p63_replay_state.strip()}")
```

**Step 3: Wire HudCockpitReplay into HudCockpitFrame**

Update `HudCockpitFrame.tsx`:
- Import `mountCockpitReplayRecorder` + `mountCockpitReplayPlayer`
- Mount both inside the component (useEffect)
- Render `<HudCockpitReplay store={store} recorder={recorder} player={player} />` next to other cockpit chrome buttons

Add wiring:
```typescript
import { useEffect, useState } from "react";
import { mountCockpitReplayRecorder, type ReplayRecorderHandle } from "../gev-visual/cockpit/replay-recorder";
import { mountCockpitReplayPlayer, type ReplayPlayerHandle } from "../gev-visual/cockpit/replay-player";
import { HudCockpitReplay } from "./HudCockpitReplay";

// inside HudCockpitFrame:
const [recorder, setRecorder] = useState<ReplayRecorderHandle | null>(null);
const [player, setPlayer] = useState<ReplayPlayerHandle | null>(null);

useEffect(() => {
  if (!state.active) return;
  const rec = mountCockpitReplayRecorder({
    getFrame: () => {
      const info = getTrackedInfo();
      return info ? {
        tMs: 0, // recorder stamps this
        frame: {
          heading: 0,
          pitchRad: 0,
          bankRad: 0,
          altitudeFt: info.altitudeM,
          speedKt: info.velocityMps,
          vsiMps: 0,
          callsign: info.callsign,
        },
      } : null;
    },
  });
  const ply = mountCockpitReplayPlayer({});
  setRecorder(rec);
  setPlayer(ply);
  return () => {
    rec.destroy();
    ply.destroy();
  };
}, [state.active, getTrackedInfo]);

// In JSX:
<HudCockpitReplay store={store} recorder={recorder} player={player} />
```

**Step 4: Run all cockpit + globe-hud tests**

```bash
cd console && npx vitest run
```

Expected: previous 695 tests still pass + ~30 new = ~725.

**Step 5: Commit + deploy**

```bash
git add console/src/gev-visual/cockpit/__tests__/source-contracts.test.ts \
        scripts/accept-sp8.py \
        console/src/globe-hud/HudCockpitFrame.tsx

git commit -m "feat(gev-6.3-t5): source contracts + sp8 checks + frame wiring

- 3 new source contracts (c13 types in barrel, c14 recorder
  exports + DB name, c15 player exports).
- 4 new sp8 bundle checks (#66-69): mountCockpitReplayRecorder,
  IndexedDB name, hud-cockpit-replay-switch testid, replayState
  field in cockpit-store.
- HudCockpitFrame mounts recorder + player inside an effect
  (only when state.active), passes them to HudCockpitReplay.
  Cleanup on unmount destroys both adapters (interval + state
  release)."
```

## Task 6: 315 + 410 deploy (per ironclad workflow)

After all 5 tasks green locally:

1. rsync to `Debian-test` (315)
2. Build on 315: `bash scripts/build-hub.sh && bash scripts/build-console.sh`
3. Run sp8/sp6/sp7/sp3 on 315 with `KEY=$(ssh Debian-test ...)`
4. If 315 green: `git add -A && git commit && git merge --no-ff
   feat/gev-section-6-3-replay && git worktree remove
   ../IntelHub-section-6-3-replay && git branch -d
   feat/gev-section-6-3-replay`
5. rsync to `IntelHub` (410)
6. Build + restart hub-core on 410 + **wait 5 min** (stampede lesson)
7. If 410 green: `git push origin main`
8. Append §6.3 Replay ops note to `AGENTS.md` (~30 lines).

## Estimated effort

- T1: ~30 LoC + ~30 min (store + tests)
- T2: ~250 LoC + ~45 min (recorder + IndexedDB + tests)
- T3: ~250 LoC + ~45 min (player + scrub + tests)
- T4: ~200 LoC + ~30 min (popover UI + CSS + tests)
- T5: ~100 LoC + ~20 min (contracts + sp8 + wiring)
- T6: ~10 min (deploy)
- **Total: ~830 LoC + ~3 hours** in 5 commits + 1 deploy commit.