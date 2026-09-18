// P7 location search adapter — React-shell seam over the vendored
// LocationSearch state machine, backed by the hub geocode proxy
// (GET /api/v1/gev/geocode, Task 2).
//
// Two mandatory searchAndFlyTo overrides keep the vendored flight path off the
// engine's page-scoped singletons, which the IntelHub shell never configures:
//   features        — the default reads applicationServices.features (the
//                     Overpass source) and would POST to a non-existent
//                     same-origin /api/overpass; we pass the all-disabled
//                     feature source instead.
//   recoverNearView — the default is annotationResolver's Overpass recovery;
//                     P7 has no annotation resolver, so it resolves null.
//
// LocationSearch's REAL constructor contract (gev-engine/src/ui/locationSearch.js)
// requires begin/isCurrent/beforeFly in addition to input/search — run() calls
// them on every search, so a bare {input, search} throws. P7 owns no shell
// navigation-authority system, so the adapter keeps its own monotonically
// increasing search generation: each run supersedes the previous one via the
// engine's isCurrent check, and beforeFly always permits the flight.
//
// NOTE: the brief expected a `disabledFeatures()` export from
// gev-boot/request-services.ts; that factory is module-private there. We consume
// the module's PUBLIC export instead and take its `features` slot — same nine
// all-disabled methods, no engine singleton involved, no edit outside this file.
import { LocationSearch } from "gev-engine/src/ui/locationSearch.js";
import { searchAndFlyTo } from "gev-engine/src/locations.js";
import { createIntelHubRequestServices } from "../gev-boot/request-services";
import type { ApiFetch } from "../gev-adapters/http";

export type SearchState = "idle" | "searching" | "found" | "missing" | "failed";

export interface LocationSearchHandle {
  run(query: string): Promise<void>;
  getState(): SearchState;
  subscribe(fn: (s: SearchState) => void): () => void;
  destroy(): void;
}

/** Engine placeSearch contract: geocode(query, {bias, signal}) →
 *  { place: {lat,lng,label,types,viewport(southwest/northeast)} | null, answered }.
 *  A transport/HTTP failure THROWS so LocationSearch lands on 'failed'; a
 *  definitive empty result returns {place:null} → 'missing'. Abort rejections
 *  are swallowed by LocationSearch's own controller check. */
export function createHubPlaceSearch(apiFetch: ApiFetch) {
  return {
    async geocode(
      query: string,
      ctx: { bias?: unknown; signal?: AbortSignal } = {},
    ) {
      const resp = await apiFetch(
        `/api/v1/gev/geocode?q=${encodeURIComponent(query)}`,
        { signal: ctx.signal },
      );
      if (!resp.ok) {
        throw new Error(`geocode failed: HTTP ${resp.status ?? "error"}`);
      }
      const body = (await resp.json()) as {
        results?: Array<Record<string, unknown>>;
      };
      return { place: body.results?.[0] ?? null, answered: true };
    },
  };
}

/** Vendor statuses have no 1:1 map for the shell's five states. A superseded
 *  or disposed search is not a result — surfacing it as 'idle' keeps the bar
 *  neutral instead of flashing "not found" for a cancelled lookup. */
const STATUS_TO_STATE: Record<string, SearchState> = {
  idle: "idle",
  searching: "searching",
  found: "found",
  missing: "missing",
  failed: "failed",
  cancelled: "idle",
  disposed: "idle",
};

export function mountLocationSearch(
  viewer: unknown,
  input: HTMLInputElement,
  apiFetch: ApiFetch,
): LocationSearchHandle {
  const placeSearch = createHubPlaceSearch(apiFetch);
  const features = createIntelHubRequestServices(apiFetch).features;
  let navigationGeneration = 0;

  const engine = new LocationSearch({
    input,
    begin: () => ++navigationGeneration,
    isCurrent: (generation: number) => generation === navigationGeneration,
    beforeFly: () => true,
    search: (
      query: string,
      options: { signal?: AbortSignal; beforeFly?: () => boolean },
    ) =>
      searchAndFlyTo(viewer, query, {
        placeSearch,
        features,
        recoverNearView: async () => null,
        // Forward the engine's own options (signal + beforeFly) AFTER our
        // overrides so caller cancellation and handoff flow unchanged.
        ...options,
      }),
  });

  const listeners = new Set<(s: SearchState) => void>();
  let lastState: SearchState = "idle";

  const snapshot = (): SearchState => {
    const raw = engine.getState() as
      | { status?: string }
      | string
      | undefined;
    const status = typeof raw === "string" ? raw : raw?.status;
    return STATUS_TO_STATE[status ?? "idle"] ?? "idle";
  };

  // emitCurrent: false — the shell's first render is not a state change.
  engine.subscribe(
    () => {
      const next = snapshot();
      if (next === lastState) return;
      lastState = next;
      for (const fn of [...listeners]) fn(next);
    },
    { emitCurrent: false },
  );

  let destroyed = false;
  return {
    run: (query: string) => engine.run(query),
    getState: () => (destroyed ? "idle" : snapshot()),
    subscribe(fn: (s: SearchState) => void) {
      if (destroyed) return () => {};
      listeners.add(fn);
      return () => {
        listeners.delete(fn);
      };
    },
    destroy() {
      if (destroyed) return;
      destroyed = true;
      listeners.clear();
      engine.destroy();
    },
  };
}
