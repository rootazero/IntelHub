// T14: CctvPopoutPanel — face-on 2D popout for CCTV camera images.
// Tests verify: image/video element rendering, close button, ESC key
// (with jsdom 26+ isTrusted workaround), attribution chip.
import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
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
});
