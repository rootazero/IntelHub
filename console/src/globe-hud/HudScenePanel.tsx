// GEV P10 T3 — scene director panel HUD.
//
// Wraps the scene-controls + scene-sharing tail adapters in a presentational
// component: a left-rail panel listing scenes/shots, with capture / share /
// close buttons. The adapters own the project state machine; this HUD owns
// the chrome (panel borders, divider, scene list rows, button bar).
//
// testids (brief §testids):
//   - `hud-scene-panel`           — outer panel container
//   - `hud-scene-capture-button`  — capture shot button
//   - `hud-scene-share-link`      — share scene link/button
//
// Visibility / state owned by parent (GlobeV2 wiring) so the panel can be
// hidden without unmounting the underlying scene-controls subscription.
import { type ReactNode } from "react";

export interface HudSceneShot {
  id: string;
  title: string;
}

export interface HudScene {
  id: string;
  title: string;
  shots: HudSceneShot[];
}

export interface HudScenePanelProps {
  /** Show/hide the panel (parent owns; HUD just hides via display). */
  visible: boolean;
  /** Project state — what to render. Empty array = "no scenes" placeholder. */
  scenes: HudScene[];
  selectedSceneId: string | null;
  selectedShotId: string | null;
  /** Whether a capture is in flight (disables capture button). */
  capturing?: boolean;
  onSelectScene?: (sceneId: string) => void;
  onSelectShot?: (sceneId: string, shotId: string) => void;
  onCapture?: () => void;
  onShare?: () => void;
  onClose?: () => void;
  /** Extra status line under the scene list (runtime/progress text). */
  status?: ReactNode;
}

export function HudScenePanel({
  visible,
  scenes,
  selectedSceneId,
  selectedShotId,
  capturing,
  onSelectScene,
  onSelectShot,
  onCapture,
  onShare,
  onClose,
  status,
}: HudScenePanelProps) {
  const selectedScene = scenes.find((s) => s.id === selectedSceneId) ?? null;
  return (
    <section
      className={`hud-scene-panel${visible ? "" : " hidden"}`}
      data-testid="hud-scene-panel"
      aria-label="场景导演 / Scene director"
      aria-hidden={!visible}
    >
      <header className="hud-scene-panel-header">
        <span className="hud-scene-panel-title">场景 / Scene</span>
        {onClose && (
          <button
            type="button"
            className="hud-scene-panel-close"
            onClick={onClose}
            aria-label="关闭场景面板 / Close"
            title="关闭 / Close"
          >
            ×
          </button>
        )}
      </header>
      <div className="hud-scene-panel-controls">
        <select
          className="hud-scene-panel-select"
          data-testid="hud-scene-select"
          value={selectedSceneId ?? ""}
          onChange={(event) => onSelectScene?.(event.target.value)}
          disabled={scenes.length === 0}
        >
          {scenes.length === 0 && <option value="">—</option>}
          {scenes.map((scene) => (
            <option key={scene.id} value={scene.id}>
              {scene.title}
            </option>
          ))}
        </select>
        <button
          type="button"
          className="hud-scene-capture-button"
          data-testid="hud-scene-capture-button"
          onClick={onCapture}
          disabled={!selectedScene || capturing}
          aria-label="捕获镜头 / Capture shot"
          title="捕获 / Capture"
        >
          ⦿
        </button>
        <button
          type="button"
          className="hud-scene-share-link"
          data-testid="hud-scene-share-link"
          onClick={onShare}
          disabled={!selectedScene}
          aria-label="分享场景 / Share scene"
          title="分享 / Share"
        >
          ⇪
        </button>
      </div>
      <ul className="hud-scene-panel-list" data-testid="hud-scene-shot-list">
        {selectedScene ? (
          selectedScene.shots.length === 0 ? (
            <li className="hud-scene-panel-empty">无镜头 / No shots</li>
          ) : (
            selectedScene.shots.map((shot) => (
              <li
                key={shot.id}
                className={`hud-scene-shot-row${
                  shot.id === selectedShotId ? " active" : ""
                }`}
              >
                <button
                  type="button"
                  onClick={() => onSelectShot?.(selectedScene.id, shot.id)}
                  aria-pressed={shot.id === selectedShotId}
                >
                  {shot.title}
                </button>
              </li>
            ))
          )
        ) : (
          <li className="hud-scene-panel-empty">未选场景 / No scene</li>
        )}
      </ul>
      {status && (
        <div className="hud-scene-panel-status" data-testid="hud-scene-status">
          {status}
        </div>
      )}
    </section>
  );
}