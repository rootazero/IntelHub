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
  getFrame(): Omit<ReplayFrame, "tMs"> | null;
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

/** Run a transaction then close the DB connection so subsequent
 *  `deleteDatabase` calls (in test teardown) don't block waiting
 *  for open connections. */
async function withDb<T>(
  mode: IDBTransactionMode,
  fn: (db: IDBDatabase) => Promise<T>,
): Promise<T> {
  const db = await openDb();
  try {
    return await fn(db);
  } finally {
    db.close();
  }
}

async function idbGetAll(): Promise<ReplaySegment[]> {
  return withDb("readonly", (db) => {
    return new Promise<ReplaySegment[]>((resolve, reject) => {
      const tx = db.transaction(STORE, "readonly");
      const store = tx.objectStore(STORE);
      const req = store.getAll();
      req.onsuccess = () => resolve((req.result ?? []) as ReplaySegment[]);
      req.onerror = () => reject(req.error);
    });
  });
}

async function idbGet(id: string): Promise<ReplaySegment | null> {
  return withDb("readonly", (db) => {
    return new Promise<ReplaySegment | null>((resolve, reject) => {
      const tx = db.transaction(STORE, "readonly");
      const req = tx.objectStore(STORE).get(id);
      req.onsuccess = () =>
        resolve((req.result ?? null) as ReplaySegment | null);
      req.onerror = () => reject(req.error);
    });
  });
}

async function idbPut(segment: ReplaySegment): Promise<void> {
  return withDb("readwrite", (db) => {
    return new Promise<void>((resolve, reject) => {
      const tx = db.transaction(STORE, "readwrite");
      const req = tx.objectStore(STORE).put(segment);
      req.onsuccess = () => resolve();
      req.onerror = () => reject(req.error);
    });
  });
}

async function idbDelete(id: string): Promise<void> {
  return withDb("readwrite", (db) => {
    return new Promise<void>((resolve, reject) => {
      const tx = db.transaction(STORE, "readwrite");
      const req = tx.objectStore(STORE).delete(id);
      req.onsuccess = () => resolve();
      req.onerror = () => reject(req.error);
    });
  });
}

export interface MountCockpitReplayRecorderDeps {
  getFrame: () => Omit<ReplayFrame, "tMs"> | null;
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
    const snap = deps.getFrame();
    if (!snap) return;
    const elapsed = frames.length * sampleMs;
    frames.push({ tMs: elapsed, frame: snap });
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
        frames: [...frames],
      };
      try {
        await idbPut(segment);
      } catch (e) {
        console.warn("[replay-recorder] IDB put failed:", e);
      }
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
      try {
        return await idbGetAll();
      } catch (e) {
        console.warn("[replay-recorder] IDB getAll failed:", e);
        return [];
      }
    },

    async loadSegment(id: string) {
      try {
        return await idbGet(id);
      } catch (e) {
        console.warn("[replay-recorder] IDB get failed:", e);
        return null;
      }
    },

    async deleteSegment(id: string) {
      try {
        await idbDelete(id);
      } catch (e) {
        console.warn("[replay-recorder] IDB delete failed:", e);
      }
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