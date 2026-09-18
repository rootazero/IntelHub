// P7 follow controller — owns "which layer is tracking which id" on behalf of
// the HUD. Tracking itself lives in the engine layers (trackById sets
// viewer.trackedEntity + applyTrackedCameraFrame internally); this adapter
// only routes to the right layer module and keeps the book.
export type FollowKind = "flight" | "satellite";

const LAYER_BY_KIND: Record<FollowKind, string> = {
  flight: "flights",
  satellite: "satellites",
};

export interface FollowHandle {
  follow(kind: FollowKind, id: string): boolean;
  unfollow(): void;
  trackedId(): string | null;
}

interface LayerModule {
  trackById(id: string | number, opts: { origin: string }): boolean;
  stopTracking(opts: { origin: string }): void;
}

export function mountFollowController(
  dataManager: { layers?: Map<string, { module: LayerModule }> },
): FollowHandle {
  if (!(dataManager?.layers instanceof Map)) {
    throw new TypeError("mountFollowController: dataManager.layers must be a Map");
  }
  let current: { kind: FollowKind; id: string } | null = null;

  const moduleFor = (kind: FollowKind): LayerModule | null =>
    dataManager.layers!.get(LAYER_BY_KIND[kind])?.module ?? null;

  return {
    follow(kind, id) {
      const module = moduleFor(kind);
      if (!module) return false;
      const arg: string | number = kind === "satellite" ? Number(id) : id;
      if (kind === "satellite" && !Number.isFinite(arg)) return false;
      const ok = module.trackById(arg, { origin: "programmatic" });
      if (!ok) return false;
      if (current && current.kind !== kind) {
        moduleFor(current.kind)?.stopTracking({ origin: "programmatic" });
      }
      current = { kind, id };
      return true;
    },
    unfollow() {
      if (!current) return;
      moduleFor(current.kind)?.stopTracking({ origin: "programmatic" });
      current = null;
    },
    trackedId: () => current?.id ?? null,
  };
}
