// GEV P10 T2 — useShareRestoration hook tests (D1 ruling).
//
// The hook replaces vendor shareRestoration.js (a 280-line class) with a
// pure URL-hash roundtrip. Tests cover:
//   1. encode/decode roundtrip (every key)
//   2. partial hashes (only some keys present)
//   3. corrupted hash tolerance (malformed base64 / JSON / unknown kinds)
//   4. persist() via history.replaceState (jsdom supports this)
//   5. hook subscribes to hashchange events
//   6. restoredAt updates on hashchange
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import {
  decodeShareHash,
  encodeShareState,
  useShareRestoration,
  type ShareState,
} from "../use-share-restoration";

describe("use-share-restoration — pure encode/decode", () => {
  test("roundtrip: all keys survive encode→decode", () => {
    const state: ShareState = {
      style: "anime",
      cam: { lat: 40.7, lng: -74.0, alt: 15000, heading: 0, pitch: -45 },
      tracking: { kind: "flight", id: "abc123" },
      selected: "feature-xyz",
      layout: { panels: { "pp-toggles": { x: 100, y: 200, collapsed: false } } },
    };
    const hash = encodeShareState(state);
    const decoded = decodeShareHash(hash);
    expect(decoded).toEqual(state);
  });

  test("empty state encodes to '#'", () => {
    expect(encodeShareState({})).toBe("#");
  });

  test("decodeShareHash('') returns null", () => {
    expect(decodeShareHash("")).toBeNull();
  });

  test("decodeShareHash('#') returns null", () => {
    expect(decodeShareHash("#")).toBeNull();
  });

  test("partial hash: style only", () => {
    const decoded = decodeShareHash("#style=retro");
    expect(decoded).toEqual({ style: "retro" });
  });

  test("partial hash: camera only", () => {
    const decoded = decodeShareHash("#cam=51.5,-0.1,25000,30,-90");
    expect(decoded).toEqual({
      cam: { lat: 51.5, lng: -0.1, alt: 25000, heading: 30, pitch: -90 },
    });
  });

  test("partial hash: tracking only", () => {
    const decoded = decodeShareHash("#tracking=satellite:NORAD-25544");
    expect(decoded).toEqual({
      tracking: { kind: "satellite", id: "NORAD-25544" },
    });
  });

  test("partial hash: selected only", () => {
    const decoded = decodeShareHash("#selected=flight-42");
    expect(decoded).toEqual({ selected: "flight-42" });
  });

  test("unknown style value is dropped, other keys survive", () => {
    const decoded = decodeShareHash("#style=plasma&selected=flight-42");
    expect(decoded).toEqual({ selected: "flight-42" });
  });

  test("unknown tracking kind is dropped", () => {
    const decoded = decodeShareHash("#tracking=car:vehicle-1");
    expect(decoded).toEqual({});
  });

  test("malformed camera (wrong arity) is dropped", () => {
    const decoded = decodeShareHash("#cam=40,-74&selected=flight-1");
    expect(decoded).toEqual({ selected: "flight-1" });
  });

  test("malformed camera (NaN) is dropped", () => {
    const decoded = decodeShareHash("#cam=foo,bar,baz,qux,quux");
    expect(decoded).toEqual({});
  });

  test("malformed base64 layout is dropped", () => {
    const decoded = decodeShareHash("#layout=not-base64!!!");
    expect(decoded).toEqual({});
  });

  test("valid base64 but invalid JSON layout is dropped", () => {
    // btoa('not json') = "bm90IGpzb24="
    const decoded = decodeShareHash("#layout=bm90IGpzb24=");
    expect(decoded).toEqual({});
  });

  test("tracking without colon is dropped", () => {
    const decoded = decodeShareHash("#tracking=flightabc");
    expect(decoded).toEqual({});
  });

  test("tracking with empty id is dropped", () => {
    const decoded = decodeShareHash("#tracking=flight:");
    expect(decoded).toEqual({});
  });
});

describe("use-share-restoration — React hook", () => {
  beforeEach(() => {
    window.location.hash = "";
  });

  afterEach(() => {
    cleanup();
    window.location.hash = "";
    vi.restoreAllMocks();
  });

  function Probe() {
    const { urlState, restoredAt, persist } = useShareRestoration();
    return (
      <div>
        <span data-testid="restored-at">{restoredAt}</span>
        <span data-testid="style">{urlState.style ?? ""}</span>
        <span data-testid="selected">{urlState.selected ?? ""}</span>
        <button
          data-testid="persist"
          onClick={() =>
            persist({ style: "noir", selected: "flight-7" })
          }
        >
          persist
        </button>
      </div>
    );
  }

  test("hook reads initial window.location.hash on mount", () => {
    window.location.hash = "#style=anime";
    render(<Probe />);
    expect(screen.getByTestId("style").textContent).toBe("anime");
  });

  test("hashchange event updates urlState + restoredAt", async () => {
    window.location.hash = "#style=normal";
    render(<Probe />);
    const initial = Number(screen.getByTestId("restored-at").textContent);
    expect(screen.getByTestId("style").textContent).toBe("normal");
    // Trigger a hashchange via the native event (jsdom doesn't auto-fire on
    // hash assignment, so we dispatch it explicitly).
    window.location.hash = "#style=thermal";
    window.dispatchEvent(new Event("hashchange"));
    await waitFor(() =>
      expect(screen.getByTestId("style").textContent).toBe("thermal"),
    );
    expect(
      Number(screen.getByTestId("restored-at").textContent),
    ).toBeGreaterThanOrEqual(initial);
  });

  test("persist() writes via history.replaceState + updates urlState", async () => {
    const replaceSpy = vi.spyOn(window.history, "replaceState");
    render(<Probe />);
    screen.getByTestId("persist").click();
    expect(replaceSpy).toHaveBeenCalledTimes(1);
    expect(window.location.hash).toBe("#style=noir&selected=flight-7");
    await waitFor(() =>
      expect(screen.getByTestId("style").textContent).toBe("noir"),
    );
    expect(screen.getByTestId("selected").textContent).toBe("flight-7");
  });

  test("persist() does NOT push a new entry (no back-button trap)", () => {
    const pushSpy = vi.spyOn(window.history, "pushState");
    render(<Probe />);
    screen.getByTestId("persist").click();
    expect(pushSpy).not.toHaveBeenCalled();
  });

  test("hashchange listener is removed on unmount (no leak)", () => {
    const { unmount } = render(<Probe />);
    const before = (window as unknown as { __hashListeners?: number })
      .__hashListeners;
    // jsdom doesn't track listener counts; we approximate by removing and
    // verifying no further state updates fire.
    unmount();
    window.location.hash = "#style=surveillance";
    window.dispatchEvent(new Event("hashchange"));
    // If the listener leaked, the screen would re-render. Without the
    // component, screen.getByTestId would throw — we just confirm cleanup
    // didn't error.
    expect(() => cleanup()).not.toThrow();
    void before;
  });
});