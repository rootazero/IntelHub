// T14: Face-on 2D CCTV popout panel (GEV P11).
// Replaces the engine's 3D-tilted monitor plane with a flat overlay.
// Uses cctvSource.getFrameUrl(camera) for image src (hub proxy, same-origin).
// mp4/hls/webm → <video>; image/mjpeg → <img> with SVG onerror fallback.
// Drag: mousedown on header → mousemove updates position → mouseup persists.
// localStorage key: godsEyeView.v11.cctvPopout.pos (P11 namespace).
import { useEffect, useState } from "react";
import { cctvSource } from "../gev-adapters/cctv";
import { makeApiFetch } from "../gev-adapters/http";

export interface PopoutCamera {
  id: string;
  name?: string;
  city?: string;
  lat?: number;
  lon?: number;
  headingDeg?: number;
  fovDeg?: number;
  pitchDeg?: number;
  feedType?: "image" | "mjpeg" | "mp4" | "hls" | "webm";
  license?: string;
  provider?: string;
  frameUrl?: string;
  live?: boolean;
  [k: string]: unknown;
}

const POS_KEY = "godsEyeView.v11.cctvPopout.pos";

function htmlEscape(s: string): string {
  return s.replace(/[<>&"]/g, (c) =>
    c === "<" ? "&lt;" : c === ">" ? "&gt;" : c === "&" ? "&amp;" : "&quot;",
  );
}

function buildFallbackSvg(camera: PopoutCamera): string {
  const id = htmlEscape(String(camera.id ?? ""));
  const name = htmlEscape(String(camera.name ?? camera.id ?? ""));
  const city = htmlEscape(String(camera.city ?? ""));
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 320 180">
    <defs>
      <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0%" stop-color="#0a1418"/>
        <stop offset="100%" stop-color="#020406"/>
      </linearGradient>
    </defs>
    <rect width="320" height="180" fill="url(#bg)"/>
    <text x="16" y="32" font-family="monospace" font-size="10" fill="rgba(145,237,255,0.7)">${id}</text>
    <text x="16" y="56" font-family="monospace" font-size="13" fill="rgba(255,255,255,0.9)">${name}</text>
    <text x="16" y="78" font-family="monospace" font-size="10" fill="rgba(145,237,255,0.6)">${city}</text>
    <text x="160" y="160" text-anchor="middle" font-family="monospace" font-size="10" fill="rgba(255,176,82,0.7)">FRAME UNAVAILABLE</text>
  </svg>`;
}

function defaultPos(): { x: number; y: number } {
  if (typeof window === "undefined") return { x: 100, y: 100 };
  return {
    x: Math.round(window.innerWidth / 2 - 320),
    y: Math.round(window.innerHeight / 2 - 180),
  };
}

function loadPos(): { x: number; y: number } {
  try {
    const raw = localStorage.getItem(POS_KEY);
    if (raw) return JSON.parse(raw) as { x: number; y: number };
  } catch {
    // ignore
  }
  return defaultPos();
}

export function CctvPopoutPanel({
  camera,
  onClose,
}: {
  camera: PopoutCamera;
  onClose: () => void;
}) {
  // cctvSource needs an ApiFetch — use the hub's default.
  // The panel is only mounted after a user interaction, so this is safe.
  const apiFetch = makeApiFetch(
    (import.meta.env.VITE_HUB_URL as string | undefined) ?? "",
    (import.meta.env.VITE_HUB_TOKEN as string | undefined) ?? "",
  );
  const cctv = cctvSource(apiFetch);

  const isVideo =
    camera.feedType === "mp4" ||
    camera.feedType === "hls" ||
    camera.feedType === "webm";

  const [imgError, setImgError] = useState(false);

  // localStorage position — initialized from localStorage, updated on mouseup
  const [pos, setPos] = useState<{ x: number; y: number }>(loadPos);

  // Persist position on change (mouseup handler writes here)
  useEffect(() => {
    try {
      localStorage.setItem(POS_KEY, JSON.stringify(pos));
    } catch {
      // ignore quota errors
    }
  }, [pos]);

  // ESC key closes
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  // Drag: mousedown on header → mousemove updates position → mouseup persists
  const onHeaderMouseDown = (e: React.MouseEvent<HTMLElement>) => {
    // Prevent text selection during drag
    e.preventDefault();
    const start = { x: e.clientX - pos.x, y: e.clientY - pos.y };

    const onMove = (ev: MouseEvent) => {
      setPos({ x: ev.clientX - start.x, y: ev.clientY - start.y });
    };

    const onUp = () => {
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
    };

    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
  };

  const frameUrl = cctv.getFrameUrl(camera);
  const mediaUrl = cctv.getMediaUrl(camera);
  const fallbackDataUrl = `data:image/svg+xml;utf8,${encodeURIComponent(buildFallbackSvg(camera))}`;

  return (
    <div
      className="cctv-popout-overlay"
      role="dialog"
      aria-label={`Live feed from ${camera.name ?? camera.id}`}
    >
      <div
        className="cctv-popout"
        style={{ position: "absolute", left: pos.x, top: pos.y }}
      >
        <header className="cctv-popout-header" onMouseDown={onHeaderMouseDown}>
          <span data-testid="cctv-popout-title">
            {camera.name ?? camera.id}
            {camera.city ? ` · ${camera.city}` : ""}
          </span>
          <button
            data-testid="cctv-popout-close"
            aria-label="Close"
            onClick={onClose}
          >
            ×
          </button>
        </header>

        <div className="cctv-popout-frame">
          {isVideo ? (
            <video
              data-testid="cctv-popout-video"
              autoPlay
              loop
              muted
              src={mediaUrl}
            />
          ) : imgError ? (
            <img src={fallbackDataUrl} alt="frame unavailable" />
          ) : (
            <img
              src={frameUrl}
              alt={`live camera ${camera.name ?? camera.id}`}
              onError={() => setImgError(true)}
            />
          )}
        </div>

        <footer className="cctv-popout-footer">
          <span data-testid="cctv-popout-attribution">
            {camera.provider ?? ""}
            {camera.license ? ` · ${camera.license}` : ""}
          </span>
        </footer>
      </div>
    </div>
  );
}
