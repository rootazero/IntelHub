// GEV P10 T3 — keyboard shortcut cheatsheet HUD.
//
// The cheatsheet pops up when the operator presses `?` (handled by the
// shortcuts tail adapter). The HUD just renders a centered card listing
// every shortcut registered on the vendor's `applicationShortcuts` module.
//
// testids (brief §testids):
//   - `hud-shortcut-cheatsheet`           — outer card
//   - `hud-shortcut-key-{name}`           — per-shortcut row (one per action)
//
// The 9 entries mirror the 8 vendor actions (setStyle/dismissSearch/toggleHud/
// toggleOrbit/toggleCleanView/toggleLayers/cycleDetection/toggleCctv) plus
// the adapter-added `?` cheatsheet pop-key. Rows render in vendor action
// order — surface layout is left to the CSS.
import { useEffect } from "react";

export interface HudShortcutEntry {
  /** Stable key (matches the {name} suffix in the testid). */
  name: string;
  /** Display combo: "1-7" for setStyle, "?" for cheatsheet, etc. */
  combo: string;
  /** Localized description (zh / en). */
  description: string;
}

/** Default 9-entry list — kept in sync with the shortcuts adapter. */
export const DEFAULT_SHORTCUTS: HudShortcutEntry[] = [
  { name: "setStyle", combo: "1-7", description: "切换地球样式 / globe style" },
  { name: "dismissSearch", combo: "Esc", description: "关闭搜索 / dismiss search" },
  { name: "toggleHud", combo: "h", description: "切换 HUD / toggle HUD" },
  { name: "toggleOrbit", combo: "o", description: "切换轨道 / toggle orbit" },
  { name: "toggleCleanView", combo: "v", description: "切换纯净视图 / clean view" },
  { name: "toggleLayers", combo: "f", description: "切换图层 / toggle layers" },
  { name: "cycleDetection", combo: "d", description: "循环检测 / cycle detection" },
  { name: "toggleCctv", combo: "c", description: "切换 CCTV / toggle CCTV" },
  { name: "toggleCheatsheet", combo: "?", description: "快捷键面板 / cheatsheet" },
];

export interface HudShortcutCheatsheetProps {
  /** Visibility (parent owns; `?` toggles via the adapter). */
  visible: boolean;
  /** Override the default 9-entry list (rare; tests pass a smaller list). */
  shortcuts?: HudShortcutEntry[];
  /** Close handler — also fires on Escape while visible. */
  onClose?: () => void;
}

export function HudShortcutCheatsheet({
  visible,
  shortcuts = DEFAULT_SHORTCUTS,
  onClose,
}: HudShortcutCheatsheetProps) {
  // Press Escape while the cheatsheet is open → close. The vendor's
  // applicationShortcuts already routes Escape to dismissSearch(); we
  // intercept it first (capture phase) so the cheatsheet closes BEFORE the
  // search-dismiss path runs (otherwise closing the cheatsheet would
  // simultaneously dismiss any active search query).
  useEffect(() => {
    if (!visible || !onClose) return;
    const onKeyDown = (event: KeyboardEvent) => {
      // Gate on isTrusted: GlobeV2.dismissSearch (shortcuts adapter)
      // synthesizes `new KeyboardEvent("keydown", {key:"Escape",bubbles:true})`
      // — those carry isTrusted=false. Letting them through here would
      // close the cheatsheet, retrigger the shortcuts effect, and recurse
      // (observed 46 RangeError pageerrors per session in sp8 P10 probe).
      // Real keyboard presses always carry isTrusted=true.
      if (!event.isTrusted) return;
      if (event.key === "Escape") {
        event.stopPropagation();
        onClose();
      }
    };
    document.addEventListener("keydown", onKeyDown, true);
    return () => document.removeEventListener("keydown", onKeyDown, true);
  }, [visible, onClose]);

  if (!visible) return null;

  return (
    <div
      className="hud-shortcut-cheatsheet"
      data-testid="hud-shortcut-cheatsheet"
      role="dialog"
      aria-modal="true"
      aria-label="快捷键面板 / Shortcut cheatsheet"
    >
      <header className="hud-shortcut-cheatsheet-header">
        <span>快捷键 / Shortcuts</span>
        {onClose && (
          <button
            type="button"
            className="hud-shortcut-cheatsheet-close"
            onClick={onClose}
            aria-label="关闭 / Close"
            title="关闭 / Close"
          >
            ×
          </button>
        )}
      </header>
      <ul className="hud-shortcut-cheatsheet-list">
        {shortcuts.map((entry) => (
          <li
            key={entry.name}
            className="hud-shortcut-cheatsheet-row"
            data-testid={`hud-shortcut-key-${entry.name}`}
          >
            <kbd className="hud-shortcut-cheatsheet-combo">{entry.combo}</kbd>
            <span className="hud-shortcut-cheatsheet-desc">
              {entry.description}
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}