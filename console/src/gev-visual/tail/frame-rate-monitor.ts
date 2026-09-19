// GEV P10 T2 — frame-rate-monitor adapter.
//
// Vendor shape (verified at console/gev-engine/src/ui/frameRateMonitor.js):
//   createFrameRateMonitor({ viewer, documentRef = document })
//     → { destroy() }
//
// Vendor behavior:
//   - Looks up #title-bar on construction; if absent, returns inert handle.
//   - Subscribes to viewer.scene.postRender for frame counts.
//   - Listens for the backtick key (`) to toggle the FPS readout.
//   - Self-displays "FPS {n}" inside a .frame-rate-readout child of #title-bar.
//   - destroy() removes the readout + listener + key handler.
//
// The adapter is a thin pass-through. The two reasons we still ship it:
//   1. Tests need a uniform Handle surface (8 of 9 adapters expose
//      destroy()); matching the contract is more important than a one-liner.
//   2. The `?` cheatsheet key (added by shortcuts.ts) coexists with the
//      vendor's backtick key — no conflict because the vendor keyset uses
//      code === 'Backquote' too. The vendor's filter excludes form
//      controls; the cheatsheet adapter does the same.
import { createFrameRateMonitor } from "gev-engine/src/ui/frameRateMonitor.js";

export interface FrameRateViewer {
  scene?: {
    postRender?: { addEventListener(cb: () => void): () => void };
  };
}

export interface FrameRateMonitorHandle {
  destroy(): void;
}

export interface FrameRateMonitorOptions {
  viewer: FrameRateViewer | null;
  documentRef?: Document;
}

export function mountFrameRateMonitor(
  opts: FrameRateMonitorOptions,
): FrameRateMonitorHandle {
  const vendor = createFrameRateMonitor({
    viewer: opts.viewer as unknown as { scene?: { postRender?: { addEventListener: (cb: () => void) => () => void } } },
    documentRef: opts.documentRef ?? (typeof document !== "undefined" ? document : undefined),
  });
  let destroyed = false;
  return {
    destroy() {
      if (destroyed) return;
      destroyed = true;
      vendor.destroy();
    },
  };
}