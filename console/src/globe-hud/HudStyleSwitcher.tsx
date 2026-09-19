// P6 style switcher — React shell over the gev-visual adapter. Labels come
// from the vendor preset table (NORMAL/CRT/NVG/FLIR/ANIME/NOIR/SNOW);
// selection persists to localStorage so a page reload restores the filter
// (GlobeV2 applies it as initialStyle on mount).
import { useEffect, useRef, useState } from "react";
import {
  GLOBE_STYLES,
  isGlobeStyle,
  type GlobeStyle,
} from "../gev-visual/visual-effects";

const STORAGE_KEY = "intelhub.globe.style";

/** Minimal surface the switcher drives. The globe style picker receives the
 *  cockpit-gated control (style-gate.ts) in GlobeV2, which is why this is a
 *  narrow setStyle-only shape instead of the full VisualEffectsHandle. */
export interface StyleSetterHandle {
  setStyle(style: GlobeStyle): void;
}

export const STYLE_LABELS: Record<GlobeStyle, string> = {
  normal: "NORMAL",
  retro: "CRT",
  surveillance: "NVG",
  thermal: "FLIR",
  anime: "ANIME",
  noir: "NOIR",
  snow: "SNOW",
};

export function readPersistedStyle(): GlobeStyle {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return isGlobeStyle(raw) ? raw : "normal";
  } catch {
    return "normal";
  }
}

export function HudStyleSwitcher({ handle }: { handle: StyleSetterHandle | null }) {
  const [open, setOpen] = useState(false);
  const [style, setStyle] = useState<GlobeStyle>(() => readPersistedStyle());
  const rootRef = useRef<HTMLDivElement | null>(null);

  // Close the menu on any click outside the switcher.
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      if (rootRef.current && !rootRef.current.contains(event.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  }, [open]);

  if (!handle) return null;

  const select = (next: GlobeStyle) => {
    handle.setStyle(next);
    setStyle(next);
    setOpen(false);
    try {
      localStorage.setItem(STORAGE_KEY, next);
    } catch {
      /* private mode: persistence is best-effort */
    }
  };

  return (
    <div className="hud-style" ref={rootRef}>
      <button
        type="button"
        className={`hud-bar-back hud-style-toggle${style !== "normal" ? " hot" : ""}`}
        onClick={() => setOpen((v) => !v)}
        title="视觉滤镜 / Visual style"
        aria-label="Visual style"
        aria-expanded={open}
        data-testid="hud-style-switcher"
      >
        ◐ {STYLE_LABELS[style]}
      </button>
      {open ? (
        <div className="hud-style-menu" role="menu">
          {GLOBE_STYLES.map((s) => (
            <button
              key={s}
              type="button"
              role="menuitemradio"
              aria-checked={s === style}
              className={`hud-style-option${s === style ? " active" : ""}`}
              onClick={() => select(s)}
              data-testid={`hud-style-option-${s}`}
            >
              {STYLE_LABELS[s]}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  );
}
