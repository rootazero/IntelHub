import "fake-indexeddb/auto";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { mountCockpitReplayRecorder } from "../replay-recorder";
import { DEFAULT_SAMPLE_INTERVAL_MS } from "../replay-types";

function fakeSnapshot(t: number) {
  return {
    heading: t / 1000,
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
    return fakeSnapshot(i);
  };
}

const sleep = (ms: number) =>
  new Promise<void>((r) => setTimeout(r, ms));

function deleteIdb(): Promise<void> {
  return new Promise((resolve) => {
    const req = indexedDB.deleteDatabase("intelhub-cockpit-replay");
    req.onsuccess = () => resolve();
    req.onerror = () => resolve(); // proceed even if delete fails
    req.onblocked = () => resolve();
  });
}

describe("mountCockpitReplayRecorder", () => {
  beforeEach(async () => {
    await deleteIdb();
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  test("starts in idle state, no segment created", () => {
    const rec = mountCockpitReplayRecorder({ getFrame: fakeFrameSource() });
    expect(rec.isRecording()).toBe(false);
    expect(rec.currentSegmentId()).toBeNull();
  });

  test("start() begins sampling", () => {
    const rec = mountCockpitReplayRecorder({ getFrame: fakeFrameSource() });
    rec.start("seg-A");
    expect(rec.isRecording()).toBe(true);
    expect(rec.currentSegmentId()).toBe("seg-A");
    rec.destroy();
  });

  test("stop() persists segment to IndexedDB", async () => {
    const rec = mountCockpitReplayRecorder({ getFrame: fakeFrameSource() });
    rec.start("seg-A");
    await sleep(250); // ~5 samples at 50ms
    const saved = await rec.stop();
    expect(saved).not.toBeNull();
    expect(saved!.id).toBe("seg-A");
    expect(saved!.frames.length).toBeGreaterThanOrEqual(3);
    expect(saved!.sampleIntervalMs).toBe(50);
    expect(rec.isRecording()).toBe(false);
  });

  test("saved segment is retrievable via listSegments", async () => {
    const rec = mountCockpitReplayRecorder({ getFrame: fakeFrameSource() });
    rec.start("seg-A");
    await sleep(200);
    await rec.stop();
    const segments = await rec.listSegments();
    expect(segments.length).toBe(1);
    expect(segments[0].id).toBe("seg-A");
  });

  test("destroy() clears sampling interval", async () => {
    const rec = mountCockpitReplayRecorder({ getFrame: fakeFrameSource() });
    rec.start("seg-A");
    rec.destroy();
    expect(rec.isRecording()).toBe(false);
    expect(rec.currentSegmentId()).toBeNull();
    // Wait to ensure no samples are taken post-destroy
    await sleep(200);
    expect(rec.isRecording()).toBe(false);
  });

  test("cancel() discards the in-progress recording", async () => {
    const rec = mountCockpitReplayRecorder({ getFrame: fakeFrameSource() });
    rec.start("seg-A");
    await sleep(200);
    rec.cancel();
    expect(rec.isRecording()).toBe(false);
    const segments = await rec.listSegments();
    expect(segments.length).toBe(0);
  });

  test("deleteSegment() removes from store", async () => {
    const rec = mountCockpitReplayRecorder({ getFrame: fakeFrameSource() });
    rec.start("seg-A");
    await sleep(150);
    await rec.stop();
    await rec.deleteSegment("seg-A");
    const segments = await rec.listSegments();
    expect(segments.length).toBe(0);
  });

  test("listSegments() returns empty array initially", async () => {
    const rec = mountCockpitReplayRecorder({ getFrame: fakeFrameSource() });
    const segments = await rec.listSegments();
    expect(segments).toEqual([]);
  });

  test("loadSegment() returns the saved segment by id", async () => {
    const rec = mountCockpitReplayRecorder({ getFrame: fakeFrameSource() });
    rec.start("seg-A");
    await sleep(150);
    await rec.stop();
    const loaded = await rec.loadSegment("seg-A");
    expect(loaded).not.toBeNull();
    expect(loaded!.id).toBe("seg-A");
  });

  test("loadSegment() returns null for unknown id", async () => {
    const rec = mountCockpitReplayRecorder({ getFrame: fakeFrameSource() });
    const loaded = await rec.loadSegment("nonexistent");
    expect(loaded).toBeNull();
  });

  test("IDB errors are swallowed (best-effort)", async () => {
    const rec = mountCockpitReplayRecorder({ getFrame: fakeFrameSource() });
    rec.start("seg-A");
    await sleep(150);
    // Force IDB put to throw; stop() should still return the segment without throwing.
    vi.spyOn(IDBObjectStore.prototype, "put").mockImplementation(() => {
      throw new Error("QuotaExceeded");
    });
    const saved = await rec.stop();
    expect(saved).not.toBeNull();
    expect(saved!.id).toBe("seg-A");
  });
});