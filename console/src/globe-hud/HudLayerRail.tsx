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
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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

export function HudLayerRail({ manager }: { manager: RailManager }) {
  const [collapsed, setCollapsed] = useState(false);
  const [openDomainId, setOpenDomainId] = useState<string | null>(null);
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
          {DOMAINS.map((domain) => (
            <button
              key={domain.id}
              type="button"
              role="menuitem"
              className={`hud-rail-icon${
                openDomainId === domain.id ? " open" : ""
              }${domainActive(domain) ? " active" : ""}`}
              title={`${domain.label.zh} · ${domain.label.en}`}
              aria-label={`${domain.label.zh} ${domain.label.en}`}
              aria-expanded={openDomainId === domain.id}
              onMouseEnter={() => openFlyout(domain.id)}
              onClick={() => toggleFlyout(domain.id)}
            >
              <span aria-hidden>{domain.icon}</span>
              {domainActive(domain) && <i className="hud-rail-dot" />}
            </button>
          ))}
        </div>
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
