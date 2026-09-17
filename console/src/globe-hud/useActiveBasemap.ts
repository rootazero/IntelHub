// T14 live basemap label: read the engine's ACTUAL map stack instead of
// guessing from VITE_GOOGLE_MAPS_KEY presence.
//
// Vendor surface (console/gev-engine/src/maps/controller.js, read-only):
//   getActiveId()     → stack id, e.g. 'photoreal' | 'esri-imagery'
//                       (MapSourceController, controller.js:44-50)
//   getActiveStack()  → descriptor {id, label} from maps/catalog.js
//                       ('Google 3D' / 'Esri Satellite' / …)
// Live changes broadcast as window CustomEvent 'gev:map-stack-changed' with
// detail = getState(status) — {activeId, activeStack, status, lastError}
// (app/scene.js:105-110; getState at controller.js:52-64). The photoreal
// tile-failure fallback path lands there too: _watchProvider re-runs
// setStack(fallback.id) and then emits 'error' (controller.js:284-310), so
// the event lane carries every effective-stack change.
//
// QUIRK: the scene's INITIAL setStack is silent (app/scene.js:113
// `{ silent: true }`) — no event fires for the first stack. The initial
// label therefore MUST be read from the handle, not from the event lane.
import { useEffect, useState } from "react";

/** Structural slice of MapStackController the hook consumes. */
export interface BasemapStack {
  getActiveId?(): string | null;
  getActiveStack?(): { id?: string; label?: string } | null;
}

/** P2-display names for the two stacks the IntelHub bootstrap can start on
 * (scene.js:99 initialStack). Other ids fall back to the engine descriptor
 * label, then a humanized id. */
const KNOWN_STACK_LABELS: Record<string, string> = {
  photoreal: "GOOGLE PHOTOREAL",
  "esri-imagery": "ESRI IMAGERY",
};

export function basemapLabelForId(
  id: string | null | undefined,
  stack?: BasemapStack | null,
): string | null {
  if (!id) return null;
  if (KNOWN_STACK_LABELS[id]) return KNOWN_STACK_LABELS[id];
  const descriptor = stack?.getActiveStack?.();
  if (descriptor?.label && descriptor.id === id) return descriptor.label;
  return id.replaceAll("-", " ").toUpperCase();
}

/** Read the current label straight off the handle (initial + event fallback). */
export function readBasemapLabel(
  stack: BasemapStack | null | undefined,
): string | null {
  if (!stack) return null;
  return basemapLabelForId(stack.getActiveId?.() ?? null, stack);
}

/** Live basemap label — null until the scene handle exists. */
export function useActiveBasemap(
  stack: BasemapStack | null | undefined,
): string | null {
  const [label, setLabel] = useState<string | null>(() =>
    readBasemapLabel(stack),
  );
  useEffect(() => {
    // Always re-read on handle (re)attachment: the initial stack is silent.
    setLabel(readBasemapLabel(stack));
    if (!stack || typeof window === "undefined") return;
    const onChange = (event: Event) => {
      const detail = (event as CustomEvent).detail as
        | { activeId?: string }
        | undefined;
      const id =
        typeof detail?.activeId === "string" && detail.activeId
          ? detail.activeId
          : stack.getActiveId?.() ?? null;
      setLabel(basemapLabelForId(id, stack));
    };
    window.addEventListener("gev:map-stack-changed", onChange);
    return () =>
      window.removeEventListener("gev:map-stack-changed", onChange);
  }, [stack]);
  return label;
}
