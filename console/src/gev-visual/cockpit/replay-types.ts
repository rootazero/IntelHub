// GEV §6.3 Replay — types + defaults. ReplaySegment is a named
// recording (samples + metadata); ReplayFrame is a single sample.

export interface ReplayFrameSnapshot {
  heading: number;
  pitchRad: number;
  bankRad: number;
  altitudeFt: number | null;
  speedKt: number | null;
  vsiMps: number;
  callsign: string;
}

export interface ReplayFrame {
  /** ms since segment start */
  tMs: number;
  /** Snapshot of cockpit instrument frame at this sample. */
  frame: ReplayFrameSnapshot;
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
  /** one of REPLAY_SPEED_OPTIONS */
  playbackSpeed: ReplaySpeed;
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