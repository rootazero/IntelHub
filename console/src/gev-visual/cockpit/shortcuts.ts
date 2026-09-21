// GEV P12 T5 — cockpit keyboard shortcuts (spec §6.E1).
//
// ONE window-level keydown listener, live only while the cockpit overlay is
// up. Four guards, each load-bearing:
//   - inactive cockpit → no-op, so P7 follow / every other page keeps its keys
//   - text-entry focus → typing in an input/textarea/contentEditable is never
//     swallowed (the console has a search box on the same route)
//   - Ctrl/Meta/Alt    → never shadow browser chords (Cmd+W, Ctrl+R, Alt+Tab)
//   - e.repeat         → hold-to-repeat must not spin the briefing rotation
import { useEffect } from "react";
import type { CockpitStore } from "./cockpit-store";
import type { BriefingHandle } from "./briefing-mount";
import {
  VISION_MODES,
  VISION_MODE_STORAGE_KEY,
  type VisionMode,
  type VisionMountHandle,
} from "./vision-mount";

export interface CockpitShortcutsOptions {
  store: CockpitStore;
  vision: VisionMountHandle | null;
  briefing: BriefingHandle | null;
  /** Shift+C — hide/show the cockpit panels (globe unobstructed). */
  onToggleHidden?: () => void;
  /** Tab — advance the briefing tab. */
  onNextTab?: () => void;
  /** Shift+Tab — retreat the briefing tab. */
  onPrevTab?: () => void;
}

/** True when the event target is a text-entry surface we must not hijack. */
function isTextEntryTarget(target: EventTarget | null): boolean {
  if (target instanceof HTMLInputElement) return true;
  if (target instanceof HTMLTextAreaElement) return true;
  if (target instanceof HTMLElement) {
    // jsdom does not implement `isContentEditable`, so the attribute walk is
    // the check that actually holds in tests (and in engines that lag on the
    // property). Both are kept: property first (cheap, authoritative), then
    // the ancestor scan.
    if (target.isContentEditable) return true;
    if (target.closest?.("[contenteditable]:not([contenteditable='false'])")) {
      return true;
    }
  }
  return false;
}

export function useCockpitShortcuts(opts: CockpitShortcutsOptions): void {
  const { store, vision, briefing, onToggleHidden, onNextTab, onPrevTab } = opts;

  useEffect(() => {
    if (!store) return;

    const handler = (e: KeyboardEvent) => {
      // Only act while the cockpit is the active surface.
      if (!store.getState().active) return;

      if (isTextEntryTarget(e.target)) return;
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      if (e.repeat) return;

      switch (e.key) {
        case "ArrowLeft":
          briefing?.prev();
          e.preventDefault();
          break;
        case "ArrowRight":
          briefing?.next();
          e.preventDefault();
          break;
        case "Escape":
          store.exit();
          e.preventDefault();
          break;
        case " ":
        case "Spacebar": // legacy pre-DOM4 spelling (old WebKit)
          if (store.getState().briefingPaused) store.resumeBriefing();
          else store.pauseBriefing();
          e.preventDefault();
          break;
        case "Tab":
          if (e.shiftKey) onPrevTab?.();
          else onNextTab?.();
          e.preventDefault();
          break;
        default: {
          const key = e.key;
          if (e.shiftKey && key.toUpperCase() === "C") {
            onToggleHidden?.();
            e.preventDefault();
            break;
          }
          // Number keys 1-5 → vision mode (VISION_MODES order is the spec's
          // 1..5 order: optical / crt / nvg / thermal / noir).
          const n = Number.parseInt(key, 10);
          if (Number.isInteger(n) && n >= 1 && n <= VISION_MODES.length) {
            const mode = VISION_MODES[n - 1] as VisionMode;
            vision?.setMode(mode);
            store.setVisionMode(mode);
            try {
              localStorage.setItem(VISION_MODE_STORAGE_KEY, mode);
            } catch {
              /* private mode: persistence is best-effort */
            }
            e.preventDefault();
          }
        }
      }
    };

    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [
    store,
    vision,
    briefing,
    onToggleHidden,
    onNextTab,
    onPrevTab,
  ]);
}
