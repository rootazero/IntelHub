// GEV P10 T2 — recording-controls adapter.
//
// Vendor shape (verified at console/gev-engine/src/ui/recordingControls.js):
//   new RecordingControls({ syncShareState })
//     .setRecordingMode(enabled, { hidePanels?, hudMode?, safeFrame? })
//     .destroy()
//
// The vendor assumes four DOM elements exist on construction:
//   #safe-frame-overlay, #safe-frame-box, #hud-layout-select, #hud-toggle
// If they don't exist, the vendor silently no-ops on layout mutations
// (rect.width<=0 filter in vendor presentation). The IntelHub adapter
// INJECTS the missing elements on mount if the brief's R8 contract demands
// it — the brief says "DOM elements by adapter created (mount 时注入 DOM)".
//
// The vendor also reads `this.hud.{getMode, getVariant, setMode, setVariant,
// visible}` at construction time. We require the consumer to supply `hud`
// in the options. If absent, the vendor throws — we mirror that contract
// by typing the field as required.
//
// HUD contract: { getMode, getVariant, setMode, setVariant, visible }.
//
// RecordingModes (per brief): "active" | "inactive"; "minimal" | "tactical".
// The vendor's `setRecordingMode(true)` reads/writes this.hud and toggles
// `body.recording-mode` (the sp8 check_44 acceptance hook).
import { RecordingControls } from "gev-engine/src/ui/recordingControls.js";

/** HUD contract the vendor reads/writes during recording mode. */
export interface RecordingHud {
  getMode(): string;
  getVariant(): string;
  setMode(mode: string): void;
  setVariant(variant: string): void;
  visible: boolean;
}

/** Recording overlay injected by adapter mount (per brief R8). */
export interface RecordingOverlayElements {
  overlay: HTMLElement;
  box: HTMLElement;
  hudToggle: HTMLElement;
  hudLayoutSelect: HTMLSelectElement;
}

export interface RecordingControlsHandle {
  readonly controls: InstanceType<typeof RecordingControls>;
  /** Vendor API: enabled=true → recording mode active. */
  setRecordingMode(enabled: boolean, options?: RecordingModeOptions): void;
  /** Idempotent destroy. */
  destroy(): void;
}

export interface RecordingModeOptions {
  hidePanels?: boolean;
  hudMode?: "off" | "minimal" | "full" | "auto";
  safeFrame?: "16:9" | "9:16";
}

export interface RecordingControlsOptions {
  /** Consumer-supplied HUD handle (REQUIRED — vendor throws if absent). */
  hud: RecordingHud;
  /** Called after each setRecordingMode invocation (vendor wiring). */
  syncShareState: () => void;
  /** Optional: provide custom overlay elements; otherwise adapter injects. */
  overlayElements?: Partial<RecordingOverlayElements>;
  /** Document ref for DOM injection (default globalThis.document). */
  documentRef?: Document;
}

/** Ensure the four vendor-required DOM elements exist; create+inject any
 *  that are missing, return the resolved map. Idempotent — repeat calls
 *  reuse existing nodes. */
function ensureOverlayElements(
  doc: Document,
  custom: Partial<RecordingOverlayElements> = {},
): { elements: RecordingOverlayElements; created: string[] } {
  // Track which element IDs the adapter CREATED (vs reused) so destroy()
  // only removes adapter-owned nodes. A consumer-supplied overlay must
  // survive destroy (it's consumer-owned).
  const created: string[] = [];

  function ensure(id: string, customEl: HTMLElement | null | undefined, factory: () => HTMLElement): HTMLElement {
    const existing = customEl ?? doc.getElementById(id);
    if (existing) return existing;
    const el = factory();
    el.id = id;
    doc.body.appendChild(el);
    created.push(id);
    return el;
  }

  const overlay = ensure(
    "safe-frame-overlay",
    custom.overlay,
    () => doc.createElement("div"),
  );
  const box = ensure("safe-frame-box", custom.box, () => {
    const el = doc.createElement("div");
    // Box is a child of the overlay per vendor convention. If we created
    // the overlay too, nest the box inside (vendor layout uses
    // getElementById('safe-frame-box') regardless of parent).
    if (!custom.overlay && !doc.getElementById("safe-frame-box")) {
      overlay.appendChild(el);
    }
    return el;
  });
  const hudToggle = ensure(
    "hud-toggle",
    custom.hudToggle,
    () => doc.createElement("button"),
  );
  const hudLayoutSelect = ensure(
    "hud-layout-select",
    custom.hudLayoutSelect,
    () => doc.createElement("select"),
  ) as HTMLSelectElement;

  return {
    elements: { overlay, box, hudToggle, hudLayoutSelect },
    created,
  };
}

export function mountRecordingControls(
  opts: RecordingControlsOptions,
): RecordingControlsHandle {
  const doc = opts.documentRef ?? (typeof document !== "undefined" ? document : null);
  // Inject the four required DOM elements if the page doesn't already mount
  // them. Vendor's listener wiring is on construction — we must inject
  // BEFORE new RecordingControls(...) runs.
  const ensured = doc
    ? ensureOverlayElements(doc, opts.overlayElements)
    : null;
  const elements = ensured?.elements ?? null;
  const createdIds = ensured?.created ?? [];

  const controls = new RecordingControls({
    syncShareState: opts.syncShareState,
  });

  // The vendor reads `this.hud` lazily inside setRecordingMode; we attach
  // it post-construction so the typed contract is satisfied.
  if (elements) {
    (controls as unknown as { _safeFrameOverlay: HTMLElement })._safeFrameOverlay =
      elements.overlay;
    (controls as unknown as { _safeFrameBox: HTMLElement })._safeFrameBox =
      elements.box;
    (controls as unknown as { _hudLayoutSelect: HTMLSelectElement })._hudLayoutSelect =
      elements.hudLayoutSelect;
    (controls as unknown as { _hudBtn: HTMLElement })._hudBtn =
      elements.hudToggle;
  }
  (controls as unknown as { hud: RecordingHud }).hud = opts.hud;

  let destroyed = false;
  return {
    controls,
    setRecordingMode(enabled, options) {
      controls.setRecordingMode(enabled, options);
    },
    destroy() {
      if (destroyed) return;
      destroyed = true;
      controls.destroy();
      // Remove only elements we CREATED (not consumer-supplied). Consumer-
      // supplied overlayElements are caller-owned and must survive destroy.
      if (doc) {
        for (const id of createdIds) {
          doc.getElementById(id)?.remove();
        }
      }
    },
  };
}