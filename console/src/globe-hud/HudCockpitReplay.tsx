// GEV §6.3 Replay popover — single toggle button + dialog with
// Record / Stop, Play / Pause, speed selector, and segment list.
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
  const [, force] = useState({});
  const rootRef = useRef<HTMLDivElement | null>(null);

  // Subscribe to store so UI mirrors state changes (mirror P17 pattern).
  useEffect(() => {
    const off = store.subscribe(() => force({}));
    return () => off();
  }, [store]);

  // Click-outside to close (mirrors P17).
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

  // Refresh segment list when popover opens.
  useEffect(() => {
    if (!open || !recorder) return;
    recorder
      .listSegments()
      .then(setSegments)
      .catch(() => setSegments([]));
  }, [open, recorder]);

  const state = store.getState();
  const replay = state.replayState;
  const isRecording = replay.isRecording;
  const isPlaying = replay.isPlaying;

  const toggleRecord = async () => {
    if (!recorder) return;
    if (isRecording) {
      const seg = await recorder.stop();
      if (seg) {
        store.stopRecording(seg.id);
        try {
          const list = await recorder.listSegments();
          setSegments(list);
        } catch {
          /* best-effort */
        }
      } else {
        store.stopRecording(replay.recordingSegmentId ?? "");
      }
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
    store.setPlaybackSpeed(speed as 0.5 | 1 | 2 | 4);
    player?.setSpeed(speed);
  };

  const handleDelete = async (id: string) => {
    if (!recorder) return;
    await recorder.deleteSegment(id);
    try {
      const list = await recorder.listSegments();
      setSegments(list);
    } catch {
      /* best-effort */
    }
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
          data-testid="hud-cockpit-replay-menu"
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
            disabled={!player || (!isPlaying && !replay.lastSavedSegmentId)}
          >
            {isPlaying ? "❚❚ PAUSE" : "▶ PLAY"}
          </button>
          <div
            className="hud-cockpit-replay-speeds"
            role="group"
            aria-label="Playback speed"
          >
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
              <div
                className="hud-cockpit-replay-empty"
                data-testid="hud-cockpit-replay-empty"
              >
                No recordings yet
              </div>
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
                    data-testid={`hud-cockpit-replay-delete-${s.id}`}
                    onClick={() => handleDelete(s.id)}
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