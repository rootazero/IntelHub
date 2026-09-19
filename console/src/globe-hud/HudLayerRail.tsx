// HUD layer rail (T9): vertical domain-icon column mounted in the HudFrame
// left slot. Hover/click a domain icon to open a flyout with that domain's
// layer checkboxes; toggles drive the engine LayerLifecycle (the data-phase
// `dataManager` from getComponents().data) via setEnabled(id, on, {origin:"user"}).
//
// Vendor API contract (verified against gev-engine/src/data/lifecycle.js —
// see task-9 report):
//   getAll()                 → [{id, name, icon, enabled, showInTogglePanel, ...}]
//   isEffectivelyEnabled(id) → boolean (in-flight transitions count as target)
//   setEnabled(id, on, opts) → Promise (per-layer serialized queue; unknown
//                              ids resolve silently — the rail never throws)
//   subscribe(cb)            → unsubscribe; cb(change) with change.layerId on
//                              visibility/refresh lifecycle events
//
// Structural typing only — the engine is plain JS (wildcard d.ts), so the
// rail depends on this minimal surface, not the engine's class type.
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import { DOMAINS } from "./domains";
import type { HudDomain } from "./domains";

export interface RailLayerInfo {
  id: string;
  name?: string;
  icon?: string;
  enabled?: boolean;
  /** Engine-internal/composite layers opt out of toggle panels (the engine's
   *  own layerPanel.js:148 skips the same flag); the rail respects it too. */
  showInTogglePanel?: boolean;
}

export interface RailManager {
  getAll(): RailLayerInfo[];
  isEffectivelyEnabled(layerId: string): boolean;
  setEnabled(
    layerId: string,
    shouldEnable: boolean,
    options?: { origin?: string },
  ): Promise<unknown>;
  subscribe(
    callback: (change: { type: string; layerId?: string }) => void,
  ): () => void;
}

/** Mouse-leave grace period before the flyout auto-closes. */
const FLYOUT_AUTO_CLOSE_MS = 3000;

export function HudLayerRail({
  manager,
  onToggleDraw,
  drawActive,
  onToggleCockpit,
  cockpitActive,
  onToggleScene,
  sceneActive,
  onToggleRecording,
  recordingActive,
  onToggleShortcuts,
  shortcutsActive,
  onToggleFps,
  fpsActive,
}: {
  manager: RailManager;
  /** P8: toggle the draw toolbar (renders the rail's draw entry button). */
  onToggleDraw?: () => void;
  /** P8: whether the draw toolbar is active (highlight the entry button). */
  drawActive?: boolean;
  /** P9: enter/exit the cockpit overlay (renders the rail's cockpit entry). */
  onToggleCockpit?: () => void;
  /** P9: whether the cockpit overlay is active (highlight the entry). */
  cockpitActive?: boolean;
  /** T3: toggle the scene director panel. */
  onToggleScene?: () => void;
  /** T3: whether the scene panel is active. */
  sceneActive?: boolean;
  /** T3: toggle recording mode. */
  onToggleRecording?: () => void;
  /** T3: whether recording mode is active. */
  recordingActive?: boolean;
  /** T3: open/close the shortcut cheatsheet (`?` shortcut). */
  onToggleShortcuts?: () => void;
  /** T3: whether the cheatsheet is open. */
  shortcutsActive?: boolean;
  /** T3: toggle the frame-rate readout visibility. */
  onToggleFps?: () => void;
  /** T3: whether the frame-rate readout is currently visible. */
  fpsActive?: boolean;
}) {
  const [collapsed, setCollapsed] = useState(false);
  const [openDomainId, setOpenDomainId] = useState<string | null>(null);
  // T14 a11y: roving-tabindex index within the menubar (one Tab stop for
  // the whole rail; arrows move, Home/End jump, Escape closes + refocuses).
  const [focusIndex, setFocusIndex] = useState(0);
  const iconRefs = useRef<Array<HTMLButtonElement | null>>([]);
  // Bumped on every manager lifecycle event so the getAll() snapshot and the
  // per-checkbox isEffectivelyEnabled() reads re-render while a toggle is
  // still in flight on the manager's per-layer serialized queue.
  const [epoch, setEpoch] = useState(0);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const layers = useMemo(
    () => manager.getAll(),
    // epoch in deps: manager identity is stable, so the event epoch is what
    // invalidates the snapshot.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [manager, epoch],
  );
  // showInTogglePanel !== false matches LayerLifecycle.getAll()'s normalization.
  const layerById = useMemo(
    () =>
      new Map(
        layers
          .filter((layer) => layer.showInTogglePanel !== false)
          .map((layer) => [layer.id, layer]),
      ),
    [layers],
  );
  const effectivelyEnabled = useCallback(
    (layerId: string) => manager.isEffectivelyEnabled(layerId),
    [manager],
  );
  const domainActive = (domain: HudDomain) =>
    domain.layers.some(
      (layerId) => layerById.has(layerId) && effectivelyEnabled(layerId),
    );

  useEffect(
    () =>
      manager.subscribe((change) => {
        if (change?.layerId) setEpoch((value) => value + 1);
      }),
    [manager],
  );

  const cancelAutoClose = () => {
    if (closeTimer.current) {
      clearTimeout(closeTimer.current);
      closeTimer.current = null;
    }
  };
  const armAutoClose = () => {
    cancelAutoClose();
    closeTimer.current = setTimeout(
      () => setOpenDomainId(null),
      FLYOUT_AUTO_CLOSE_MS,
    );
  };

  // Never leave a live timer across unmount.
  useEffect(() => cancelAutoClose, []);

  const openFlyout = (domainId: string) => {
    cancelAutoClose();
    setOpenDomainId(domainId);
  };
  const toggleFlyout = (domainId: string) => {
    cancelAutoClose();
    setOpenDomainId((current) => (current === domainId ? null : domainId));
  };

  // ---- rail keyboard model (T14 a11y, review r1) ---------------------------
  // role=menubar/menuitem already marked (T9). Roving tabindex: only the
  // icon at focusIndex is a Tab stop; Arrow keys move focus AND open that
  // domain's flyout (mirrors the mouseenter lane); Home/End jump; Escape
  // closes the flyout and returns focus to the icon that opened it.
  //
  // The keydown handler lives on the OUTER .hud-rail container, NOT the
  // menubar div: the flyout (a menubar sibling holding the layer
  // checkboxes) bubbles its key events up here, so Escape/arrows keep
  // working after the operator Tabs into the flyout — without this the
  // flyout was a keyboard trap.
  const focusIcon = (index: number) => {
    const clamped = (index + DOMAINS.length) % DOMAINS.length;
    setFocusIndex(clamped);
    iconRefs.current[clamped]?.focus();
  };

  const onRailKeyDown = (event: ReactKeyboardEvent) => {
    switch (event.key) {
      case "ArrowDown":
      case "ArrowRight": {
        event.preventDefault();
        const next = (focusIndex + 1) % DOMAINS.length;
        openFlyout(DOMAINS[next].id);
        focusIcon(next);
        break;
      }
      case "ArrowUp":
      case "ArrowLeft": {
        event.preventDefault();
        const prev = (focusIndex - 1 + DOMAINS.length) % DOMAINS.length;
        openFlyout(DOMAINS[prev].id);
        focusIcon(prev);
        break;
      }
      case "Home": {
        event.preventDefault();
        openFlyout(DOMAINS[0].id);
        focusIcon(0);
        break;
      }
      case "End": {
        event.preventDefault();
        const last = DOMAINS.length - 1;
        openFlyout(DOMAINS[last].id);
        focusIcon(last);
        break;
      }
      case "Escape": {
        if (!openDomainId) break;
        event.preventDefault();
        cancelAutoClose();
        setOpenDomainId(null);
        // Focus return: the trigger of the flyout being closed, so a keyboard
        // user lands back where they were before opening it.
        const triggerIndex = DOMAINS.findIndex(
          (domain) => domain.id === openDomainId,
        );
        const target = triggerIndex >= 0 ? triggerIndex : focusIndex;
        setFocusIndex(target);
        iconRefs.current[target]?.focus();
        break;
      }
    }
  };

  const onToggleLayer = (layerId: string, next: boolean) => {
    // Fire-and-forget: the per-layer serialized queue owns ordering; the
    // subscribe() epoch bump above converges the checkbox when it settles.
    // setEnabled() rejects only on queue-internal errors — keep them visible.
    void manager
      .setEnabled(layerId, next, { origin: "user" })
      .catch((error) => console.warn(`[hud] toggle ${layerId} failed:`, error));
  };

  const openDomain =
    openDomainId === null
      ? null
      : (DOMAINS.find((domain) => domain.id === openDomainId) ?? null);

  return (
    <div
      className={`hud-rail${collapsed ? " collapsed" : ""}`}
      data-testid="hud-layer-rail"
      onKeyDown={onRailKeyDown}
      onMouseLeave={armAutoClose}
      onMouseEnter={cancelAutoClose}
    >
      <button
        type="button"
        className="hud-rail-handle"
        title={collapsed ? "展开图层栏" : "折叠图层栏"}
        aria-label={collapsed ? "展开图层栏" : "折叠图层栏"}
        aria-expanded={!collapsed}
        onClick={() => setCollapsed((value) => !value)}
      >
        {collapsed ? "▸" : "◂"}
      </button>
      {!collapsed && (
        <div className="hud-rail-icons" role="menubar" aria-label="图层域">
          {DOMAINS.map((domain, index) => (
            <button
              key={domain.id}
              type="button"
              role="menuitem"
              ref={(node) => {
                iconRefs.current[index] = node;
              }}
              tabIndex={index === focusIndex ? 0 : -1}
              className={`hud-rail-icon${
                openDomainId === domain.id ? " open" : ""
              }${domainActive(domain) ? " active" : ""}`}
              title={`${domain.label.zh} · ${domain.label.en}`}
              aria-label={`${domain.label.zh} ${domain.label.en}`}
              aria-expanded={openDomainId === domain.id}
              onFocus={() => setFocusIndex(index)}
              onMouseEnter={() => openFlyout(domain.id)}
              onClick={() => toggleFlyout(domain.id)}
            >
              <span aria-hidden>{domain.icon}</span>
              {domainActive(domain) && <i className="hud-rail-dot" />}
            </button>
          ))}
        </div>
      )}
      {!collapsed && onToggleDraw && (
        <button
          type="button"
          data-testid="hud-draw-button"
          className={`hud-rail-icon hud-rail-tool${
            drawActive ? " active" : ""
          }`}
          onClick={onToggleDraw}
          title="绘制标注 / Draw"
          aria-label="绘制标注 / Draw annotation"
          aria-pressed={drawActive}
        >
          <span aria-hidden>✏️</span>
        </button>
      )}
      {!collapsed && onToggleCockpit && (
        <button
          type="button"
          data-testid="hud-cockpit-button"
          className={`hud-rail-icon hud-rail-tool${
            cockpitActive ? " active" : ""
          }`}
          onClick={onToggleCockpit}
          title="驾驶舱 / Cockpit"
          aria-label="驾驶舱 / Cockpit"
          aria-pressed={cockpitActive}
        >
          <span aria-hidden>🎮</span>
        </button>
      )}
      {!collapsed && onToggleScene && (
        <button
          type="button"
          data-testid="hud-scene-toggle"
          className={`hud-rail-icon hud-rail-tool${
            sceneActive ? " active" : ""
          }`}
          onClick={onToggleScene}
          title="场景 / Scene"
          aria-label="场景 / Scene"
          aria-pressed={sceneActive}
        >
          <span aria-hidden>🎬</span>
        </button>
      )}
      {!collapsed && onToggleRecording && (
        <button
          type="button"
          data-testid="hud-recording-button"
          className={`hud-rail-icon hud-rail-tool${
            recordingActive ? " active" : ""
          }`}
          onClick={onToggleRecording}
          title="录制 / Recording"
          aria-label="录制 / Recording"
          aria-pressed={recordingActive}
        >
          <span aria-hidden>⏺</span>
        </button>
      )}
      {!collapsed && onToggleShortcuts && (
        <button
          type="button"
          data-testid="hud-shortcuts-toggle"
          className={`hud-rail-icon hud-rail-tool${
            shortcutsActive ? " active" : ""
          }`}
          onClick={onToggleShortcuts}
          title="快捷键 / Shortcuts"
          aria-label="快捷键 / Shortcuts"
          aria-pressed={shortcutsActive}
        >
          <span aria-hidden>?</span>
        </button>
      )}
      {!collapsed && onToggleFps && (
        <button
          type="button"
          data-testid="hud-fps-toggle"
          className={`hud-rail-icon hud-rail-tool${
            fpsActive ? " active" : ""
          }`}
          onClick={onToggleFps}
          title="帧率 / FPS"
          aria-label="帧率 / FPS"
          aria-pressed={fpsActive}
        >
          <span aria-hidden>FPS</span>
        </button>
      )}
      {openDomain && !collapsed && (
        <div className="hud-rail-flyout" data-domain={openDomain.id}>
          <div className="hud-rail-flyout-title">
            {openDomain.icon} {openDomain.label.zh}
            <span className="hud-rail-flyout-sub">{openDomain.label.en}</span>
          </div>
          <ul>
            {openDomain.layers
              .filter((layerId) => layerById.has(layerId))
              .map((layerId) => {
                const layer = layerById.get(layerId)!;
                const enabled = effectivelyEnabled(layerId);
                return (
                  <li key={layerId}>
                    <label className={enabled ? "on" : ""}>
                      <input
                        type="checkbox"
                        checked={enabled}
                        onChange={(event) =>
                          onToggleLayer(layerId, event.target.checked)
                        }
                      />
                      <span className="hud-rail-layer-icon" aria-hidden>
                        {layer.icon ?? "·"}
                      </span>
                      <span className="hud-rail-layer-name">
                        {layer.name ?? layerId}
                      </span>
                    </label>
                  </li>
                );
              })}
          </ul>
        </div>
      )}
    </div>
  );
}
