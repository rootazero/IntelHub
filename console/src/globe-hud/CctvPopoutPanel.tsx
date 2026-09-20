// T14: Face-on 2D CCTV popout panel (GEV P11).
// Replaces the engine's 3D-tilted monitor plane with a flat overlay.
// Uses cctvSource.getFrameUrl(camera) for image src (hub proxy, same-origin).
// mp4/hls/webm → <video>; image/mjpeg → <img> with SVG onerror fallback.
// Drag: mousedown on header → mousemove updates position → mouseup persists.
// localStorage key: godsEyeView.v11.cctvPopout.pos (P11 namespace).
//
// P12 follow-up (2026-09-20): the popout's <img> src MUST re-tick every
// ACTIVE_FRAME_REFRESH_MS (10s) so the user sees live frames, not a
// frozen snapshot from the moment the camera was clicked. The vendor
// engine's 3D projection plane does this via
// refreshProjectionImage() in cctv/frames.js — every refreshMs it
// re-calls frameUrlFor() which embeds a fresh floor(now/refreshMs) ts
// query, so the browser's <img> cache buster fires. The popout mirrors
// the same pattern with setInterval: see the liveRefresh effect below.
import { useEffect, useState } from "react";
import { cctvSource } from "../gev-adapters/cctv";
import { makeApiFetch } from "../gev-adapters/http";

/**
 * Refresh cadence for still-image cameras in the popout panel. Mirrors
 * the vendor engine's ACTIVE_FRAME_REFRESH_MS (sourcePolicy.js) so the
 * 3D plane and the face-on popout tick together. Video feeds stream
 * natively and don't need this.
 */
const POPOUT_FRAME_REFRESH_MS = 10_000;

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
  /**
   * Direct upstream video URL (mp4/hls/webm) when the camera has one.
   * Prefer this over the hub proxy for video feeds: the popout is a
   * normal `<video>` element, so cross-origin CORS works (TfL S3 sends
   * `Access-Control-Allow-Origin: *`), and we save a hub hop + skip the
   * proxy's 4-concurrency cap (gev_cctv.rs::media_proxy_stream). Image
   * cameras (Caltrans/TxDOT/Austin JPEG) still go through the proxy.
   * Sourced from cctv_cameras.media_url via the catalog endpoint
   * (gev_cctv.rs::camera_source_json — must include `mediaUrl`).
   */
  mediaUrl?: string;
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
  // Some providers (NY511/Skyline) have real HLS video streams
  // (cctv_cameras.media_url ending in .m3u8) but the catalog tags
  // them as feed_type=image because the engine's cctv projection
  // plane treats HLS the same as image (stateless texture frame).
  // The popout panel is a real <video> element, so we can play HLS
  // natively. Use mediaUrl's extension as the ground truth.
  const upstreamMediaUrl = camera.mediaUrl;
  const hasUpstreamVideo =
    !!upstreamMediaUrl &&
    /\.(m3u8|mp4|webm)(\?|$)/i.test(upstreamMediaUrl);
  const useVideoElement = isVideo || hasUpstreamVideo;

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

  // P12 follow-up: live-refresh tick for still-image cameras.
  //
  // cctvSource.getFrameUrl() embeds a `ts=floor(Date.now()/refreshMs)` query
  // — the engine's `ts` tick that gates the proxy's Redis cache. Without
  // re-invoking it, the <img> src is frozen at the moment the panel opened
  // and the user sees a stale snapshot forever (the 2026-09-20 bug).
  //
  // Mirrors the vendor engine's cctv/frames.js::refreshProjectionImage():
  // every refreshMs we re-call frameUrlFor() to produce a fresh ts, then
  // bump a state counter so React re-renders the <img src={...}> with the
  // new URL. The browser's <img> cache treats a different `?ts=` as a new
  // fetch, so the upstream is hit again.
  //
  // Video feeds (mp4/hls/webm) stream natively — no refresh needed, the
  // <video> element loops itself.
  const [frameTick, setFrameTick] = useState(0);
  useEffect(() => {
    if (useVideoElement) return;
    const id = window.setInterval(() => {
      setFrameTick((t) => t + 1);
    }, POPOUT_FRAME_REFRESH_MS);
    return () => window.clearInterval(id);
  }, [useVideoElement]);

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

  // frameTick is referenced by the <img>/<video> key prop (below) so React
  // re-renders the entire element every refresh interval. getFrameUrl() /
  // getMediaUrl() invoke frameUrlFor() / mediaUrlFor() which embed a fresh
  // `ts=floor(Date.now()/refreshMs)` tick, so the URL is genuinely different
  // each tick and the browser re-fetches. Without this re-render the <img>
  // would never reload after mount (the 2026-09-20 frozen-snapshot bug).
  const frameUrl = cctv.getFrameUrl(camera);
  // For video feeds: prefer the upstream mediaUrl when present (real
  // H.264 stream, browser-native `<video>` handles CORS + looping
  // natively). Fall back to the hub proxy when the catalog hasn't
  // returned a mediaUrl — the proxy can still serve the same bytes via
  // its 4-concurrency-capped T12 path (gev_cctv.rs::media_proxy_stream).
  const mediaUrl =
    useVideoElement && upstreamMediaUrl ? upstreamMediaUrl : cctv.getMediaUrl(camera);
  const fallbackDataUrl = `data:image/svg+xml;utf8,${encodeURIComponent(buildFallbackSvg(camera))}`;

  return (
    <div
      className="cctv-popout-overlay"
      role="dialog"
      aria-label={`Live feed from ${camera.name ?? camera.id}`}
    >
      <div
        className="cctv-popout"
        data-testid="cctv-popout-panel"
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
          {useVideoElement ? (
            <video
              key={`v-${frameTick}`}
              data-testid="cctv-popout-video"
              autoPlay
              loop
              muted
              src={mediaUrl}
            />
          ) : imgError ? (
            <img key={`fb-${frameTick}`} src={fallbackDataUrl} alt="frame unavailable" />
          ) : (
            <img
              key={`img-${frameTick}`}
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
