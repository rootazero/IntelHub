// GEV P14 — cockpit model-visibility adapter.
//
// Hides the tracked aircraft model while cockpit is active so the cockpit
// frame is "sky + HUD + instruments", not "plane in the middle + cockpit
// chrome". Restores prior visibility on exit. Subscribe-once-then-cleanup
// pattern, mirrors the existing camera-transition.ts baseline-capture.

import type { CockpitStore } from "./cockpit-store";

/** Subset of Cesium.Entity we touch. `show` is the visibility flag. */
interface VisibilityEntity {
  show: boolean;
}

/** Narrower than the full CockpitStore — model-visibility only uses
 *  `getState` + `subscribe`. Lets tests supply a fake without implementing
 *  the action methods. */
export interface ModelVisibilityStoreShape {
  getState(): { active: boolean; trackedId: string | null };
  subscribe(fn: () => void): () => void;
}

export interface ModelVisibilityDeps {
  store: ModelVisibilityStoreShape;
  getTrackedEntity: () => VisibilityEntity | null;
}

export interface ModelVisibilityHandle {
  destroy(): void;
}

export function mountModelVisibility(
  deps: ModelVisibilityDeps,
): ModelVisibilityHandle {
  // Constructor contract: assert the seam at mount so a lenient mock cannot
  // hide a wrong object handed in later.
  if (typeof deps?.store?.subscribe !== "function") {
    throw new TypeError(
      "mountModelVisibility: deps.store.subscribe must be a function",
    );
  }
  if (typeof deps.getTrackedEntity !== "function") {
    throw new TypeError(
      "mountModelVisibility: deps.getTrackedEntity must be a function",
    );
  }

  // Remember the entity we hid so we can restore it even if followRef swaps
  // the tracked entity while cockpit is active (selection change mid-flight).
  let priorShow: boolean | null = null;
  let hiddenEntity: VisibilityEntity | null = null;

  function applyHidden(entity: VisibilityEntity): void {
    if (hiddenEntity === entity) return; // already hidden this entity
    if (hiddenEntity && hiddenEntity !== entity) {
      // Race: entity changed under us. Restore old, then hide new.
      hiddenEntity.show = priorShow ?? true;
    }
    priorShow = entity.show;
    entity.show = false;
    hiddenEntity = entity;
  }

  function restoreIfHidden(): void {
    if (hiddenEntity) {
      hiddenEntity.show = priorShow ?? true;
      hiddenEntity = null;
      priorShow = null;
    }
  }

  function onChange(): void {
    const { active } = deps.store.getState();
    if (active) {
      const entity = deps.getTrackedEntity();
      if (entity) applyHidden(entity);
    } else {
      restoreIfHidden();
    }
  }

  const unsubscribe = deps.store.subscribe(onChange);

  return {
    destroy() {
      unsubscribe();
      restoreIfHidden();
    },
  };
}
