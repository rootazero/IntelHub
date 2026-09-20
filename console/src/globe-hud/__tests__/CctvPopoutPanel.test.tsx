// T14: CctvPopoutPanel — face-on 2D popout for CCTV camera images.
// Tests verify: image/video element rendering, close button, ESC key
// (with jsdom 26+ isTrusted workaround), attribution chip,
// AND the P12 live-refresh bug fix — the <img> must update its src every
// ACTIVE_FRAME_REFRESH_MS tick so the popout shows live frames, not a
// frozen snapshot from the moment the user clicked the camera.
import { fireEvent, render, screen, act } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import "@testing-library/jest-dom/vitest";
import { CctvPopoutPanel } from "../CctvPopoutPanel";

// jsdom 26+ workaround for isTrusted: constructs a fake KeyboardEvent from the
// prototype (bypasses the constructor lockdown) and uses Object.defineProperty
// so every property is explicitly writable. Mirrors hud-shortcut-cheatsheet.test.tsx.
function makeFakeTrustedKeyboardEvent(
  init: Partial<KeyboardEventInit> & { isTrusted?: boolean } = {},
): KeyboardEvent {
  const fake = Object.create(KeyboardEvent.prototype) as Record<string, unknown>;
  Object.defineProperty(fake, "type", { value: "keydown" });
  Object.defineProperty(fake, "key", { value: init.key ?? "Escape" });
  Object.defineProperty(fake, "code", { value: init.code ?? "Escape" });
  Object.defineProperty(fake, "isTrusted", {
    get() { return init.isTrusted ?? true; },
    configurable: true,
  });
  Object.defineProperty(fake, "bubbles", { value: init.bubbles ?? true });
  Object.defineProperty(fake, "cancelable", { value: init.cancelable ?? true });
  Object.defineProperty(fake, "target", { value: null });
  Object.defineProperty(fake, "currentTarget", { value: null });
  Object.defineProperty(fake, "defaultPrevented", { value: false });
  Object.defineProperty(fake, "stopPropagation", { value: () => undefined });
  Object.defineProperty(fake, "stopImmediatePropagation", { value: () => undefined });
  Object.defineProperty(fake, "preventDefault", { value: () => undefined });
  return fake as unknown as KeyboardEvent;
}

// Minimal camera fixture matching the context-bridge cctv data shape
// (base: id/lat/lon; properties spread as flat fields).
const baseCamera = {
  id: "cctv-1",
  name: "QEW West of Thompson Road",
  city: "Ontario",
  lat: 42.91,
  lon: -78.96,
  headingDeg: 90,
  fovDeg: 74,
  pitchDeg: -17,
  feedType: "image" as const,
  license: "Powered by RWIS Open Data",
  provider: "RWIS (MTO)",
  frameUrl: "/api/v1/gev/cctv/frame/cctv-1",
  live: true,
};

afterEach(() => {
  localStorage.clear();
  vi.useRealTimers();
});

describe("CctvPopoutPanel", () => {
  it("renders image with frame URL from cctvSource", () => {
    render(<CctvPopoutPanel camera={baseCamera} onClose={vi.fn()} />);
    const img = screen.getByRole("img");
    expect(img).toHaveAttribute("src", expect.stringContaining("/api/v1/gev/cctv/frame/cctv-1"));
  });

  it("renders video element for mp4 feedType", () => {
    const mp4Cam = { ...baseCamera, feedType: "mp4" as const };
    render(<CctvPopoutPanel camera={mp4Cam} onClose={vi.fn()} />);
    expect(screen.getByTestId("cctv-popout-video")).toBeInTheDocument();
  });

  // P12 follow-up: mp4 cameras with an upstream mediaUrl MUST stream
  // from that URL (real H.264 video), not the hub proxy. The vendor
  // engine uses the hub proxy because it needs same-origin for canvas
  // texture upload; the popout panel is a normal <video> element so it
  // can take the upstream URL directly — saves a hub hop and avoids the
  // proxy's 4-concurrency cap (gev_cctv.rs::media_proxy_stream).
  it("uses upstream mediaUrl directly when provided for mp4 cameras", () => {
    const mp4Cam = {
      ...baseCamera,
      feedType: "mp4" as const,
      mediaUrl: "https://s3-eu-west-1.amazonaws.com/jamcams.tfl.gov.uk/00001.01251.mp4",
    };
    render(<CctvPopoutPanel camera={mp4Cam} onClose={vi.fn()} />);
    const v = screen.getByTestId("cctv-popout-video");
    expect(v).toBeInTheDocument();
    // src attribute must be the upstream URL (not the hub proxy /media/).
    expect(v.getAttribute("src")).toBe(mp4Cam.mediaUrl);
  });

  it("falls back to hub proxy mediaUrl when upstream mediaUrl is absent for mp4", () => {
    const mp4Cam = { ...baseCamera, feedType: "mp4" as const };  // no mediaUrl
    render(<CctvPopoutPanel camera={mp4Cam} onClose={vi.fn()} />);
    const v = screen.getByTestId("cctv-popout-video");
    // No upstream URL — must use hub proxy (which is what getMediaUrl returns).
    expect(v.getAttribute("src")).toMatch(/\/api\/v1\/gev\/cctv\/media\//);
  });

  // P12 follow-up: NY511 cameras (1565) are catalog-tagged as
  // feed_type=image but their actual feed is HLS (.m3u8) per
  // cctv_cameras.media_url. The popout must detect the .m3u8
  // extension and play real video, not show a frozen JPEG.
  it("treats HLS mediaUrl as video even when feedType=image (NY511)", () => {
    const hlsCam = {
      ...baseCamera,
      feedType: "image" as const,
      mediaUrl: "https://s53.nysdot.skyvdn.com/rtplive/R3_030/playlist.m3u8",
    };
    render(<CctvPopoutPanel camera={hlsCam} onClose={vi.fn()} />);
    const v = screen.getByTestId("cctv-popout-video");
    expect(v).toBeInTheDocument();
    expect(v.getAttribute("src")).toBe(hlsCam.mediaUrl);
  });

  it("close button calls onClose", () => {
    const onClose = vi.fn();
    render(<CctvPopoutPanel camera={baseCamera} onClose={onClose} />);
    fireEvent.click(screen.getByTestId("cctv-popout-close"));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("ESC key calls onClose with a trusted KeyboardEvent", () => {
    // Capture the listener registered by the component's useEffect, then invoke
    // it directly with a fake trusted KeyboardEvent. This bypasses jsdom's
    // fireEvent path (which re-creates the event and stamps isTrusted=false) and
    // mirrors the pattern in hud-shortcut-cheatsheet.test.tsx.
    let capturedCb: ((e: Event) => void) | null = null;
    const addSpy = vi.spyOn(document, "addEventListener").mockImplementation(
      (type: string, cb: Parameters<typeof document.addEventListener>[1]) => {
        if (type === "keydown") capturedCb = cb as (e: Event) => void;
      },
    );
    const onClose = vi.fn();
    render(<CctvPopoutPanel camera={baseCamera} onClose={onClose} />);
    addSpy.mockRestore();
    expect(capturedCb).not.toBeNull();
    capturedCb!(makeFakeTrustedKeyboardEvent({ key: "Escape" }));
    expect(onClose).toHaveBeenCalled();
  });

  it("displays attribution chip with provider and license", () => {
    render(<CctvPopoutPanel camera={baseCamera} onClose={vi.fn()} />);
    const chip = screen.getByTestId("cctv-popout-attribution");
    expect(chip.textContent).toContain("RWIS (MTO)");
    expect(chip.textContent).toContain("Powered by RWIS Open Data");
  });

  // ---- P12 follow-up: live refresh (the frozen-snapshot bug) ----

  // The user's report (2026-09-20):
  //   "显式视频画面了，但是不是视频流，而是某一时刻的静态画面。我等了几分钟，也没有更新"
  // Root cause: the popout set <img src={frameUrl}> ONCE on mount; the ts
  // query param was computed at render time and frozen. Mirror the vendor
  // engine's 3D-plane behavior (frames.js: refreshProjectionImage re-sets
  // runtime.image.src with a fresh ts tick every refreshMs).

  it("re-renders <img> src with a fresh ts after each refresh tick", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(1_700_000_000_000));

    render(<CctvPopoutPanel camera={baseCamera} onClose={vi.fn()} />);
    const img = screen.getByRole("img");
    const firstSrc = img.getAttribute("src") || "";
    expect(firstSrc).toMatch(/[?&]ts=\d+/);
    const firstTs = Number(new URLSearchParams(firstSrc.split("?")[1]).get("ts"));

    // Advance just past one 10s tick; setInterval callback fires,
    // setFrameTick advances, React re-renders, getFrameUrl re-runs with
    // a fresh Date.now() so the ts query bumps.
    act(() => {
      vi.advanceTimersByTime(10_001);
    });

    const newSrc = screen.getByRole("img").getAttribute("src") || "";
    const newTs = Number(new URLSearchParams(newSrc.split("?")[1]).get("ts"));
    expect(newTs).toBeGreaterThan(firstTs);
  });

  it("uses setInterval on ACTIVE_FRAME_REFRESH_MS cadence for still images", () => {
    vi.useFakeTimers();
    // Pin the clock to a fresh 10s-bucket boundary so the +10s advance is
    // guaranteed to cross it (otherwise the test races on system time).
    vi.setSystemTime(new Date(1_700_000_000_000));
    const setIntervalSpy = vi.spyOn(globalThis, "setInterval");

    render(<CctvPopoutPanel camera={baseCamera} onClose={vi.fn()} />);
    const img = screen.getByRole("img");
    const initialSrc = img.getAttribute("src") || "";
    const initialTick = Number(
      new URLSearchParams(initialSrc.split("?")[1]).get("ts"),
    );

    // One full refresh cycle (10s) — wrapped in act() so React flushes
    // the state update from the setInterval callback.
    act(() => {
      vi.advanceTimersByTime(10_001);
    });
    const afterSrc = screen.getByRole("img").getAttribute("src") || "";
    const afterTick = Number(
      new URLSearchParams(afterSrc.split("?")[1]).get("ts"),
    );

    expect(afterTick).toBeGreaterThan(initialTick);

    // The interval must be set with the 10s cadence so the user's
    // popout refreshes at the same cadence as the engine's 3D plane.
    const matches = setIntervalSpy.mock.calls.filter(
      ([, ms]) => ms === 10_000,
    );
    expect(matches.length).toBeGreaterThan(0);

    setIntervalSpy.mockRestore();
  });

  it("clears the refresh interval when the panel unmounts", () => {
    vi.useFakeTimers();
    const clearSpy = vi.spyOn(globalThis, "clearInterval");

    const { unmount } = render(
      <CctvPopoutPanel camera={baseCamera} onClose={vi.fn()} />,
    );
    unmount();

    expect(clearSpy).toHaveBeenCalled();
    clearSpy.mockRestore();
  });
});
