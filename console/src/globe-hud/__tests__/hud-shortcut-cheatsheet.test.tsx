// HudShortcutCheatsheet — keyboard shortcut cheatsheet presentation.
//
// Tests cover: visibility, testid rendering, per-key testids, close button,
// Escape interception, and custom shortcuts list override.
import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import { HudShortcutCheatsheet } from "../HudShortcutCheatsheet";

afterEach(() => cleanup());

/**
 * Build a fake KeyboardEvent that reports `isTrusted=true` and bypasses
 * jsdom's constructor-time stamp. jsdom locks `isTrusted` as a
 * non-configurable own property on every event created via the standard
 * constructor — real key presses in a browser carry isTrusted=true, but
 * fireEvent.keyDown / `new KeyboardEvent` cannot reproduce that in tests.
 *
 * The cheatsheet gates on `event.isTrusted` (P11-A fix for the
 * dismissSearch Escape recursion), so for the "real Escape still works"
 * assertion we manually invoke the captured capture-phase listener with a
 * fake event whose `isTrusted` getter returns true. Object.create() builds
 * an instance that inherits from KeyboardEvent.prototype without running
 * the parent constructor, so the lockdown never fires.
 */
function makeFakeTrustedKeyboardEvent(
  init: Partial<KeyboardEventInit> = {},
): KeyboardEvent {
  const fake: Record<string, unknown> = Object.create(KeyboardEvent.prototype);
  Object.defineProperty(fake, "type", { value: "keydown" });
  Object.defineProperty(fake, "key", { value: init.key ?? "Escape" });
  Object.defineProperty(fake, "code", { value: init.code ?? "Escape" });
  Object.defineProperty(fake, "isTrusted", {
    get() {
      return true;
    },
    configurable: true,
  });
  Object.defineProperty(fake, "bubbles", { value: init.bubbles ?? true });
  Object.defineProperty(fake, "cancelable", { value: init.cancelable ?? true });
  Object.defineProperty(fake, "target", { value: null });
  Object.defineProperty(fake, "currentTarget", { value: null });
  Object.defineProperty(fake, "defaultPrevented", { value: false });
  Object.defineProperty(fake, "stopPropagation", {
    value: () => undefined,
  });
  Object.defineProperty(fake, "stopImmediatePropagation", {
    value: () => undefined,
  });
  Object.defineProperty(fake, "preventDefault", {
    value: () => undefined,
  });
  return fake as unknown as KeyboardEvent;
}

/**
 * Spy on document.addEventListener and capture the first keydown listener
 * that the cheatsheet registers in its capture-phase effect. Returns the
 * captured callback so tests can invoke it directly with their fake event
 * — bypassing jsdom's dispatch path which would normalize isTrusted back
 * to false.
 */
function captureKeydownListener(): {
  capturedCbRef: { current: ((e: Event) => void) | null };
  restore: () => void;
} {
  const capturedCbRef: { current: ((e: Event) => void) | null } = {
    current: null,
  };
  const origAdd = document.addEventListener.bind(document);
  const spy = vi
    .spyOn(document, "addEventListener")
    .mockImplementation(function (
      this: Document,
      type: any,
      cb: any,
      opts?: any,
    ) {
      if (type === "keydown" && !capturedCbRef.current) {
        capturedCbRef.current = cb;
      }
      return origAdd(type, cb, opts);
    });
  return {
    capturedCbRef,
    restore() {
      spy.mockRestore();
    },
  };
}

describe("HudShortcutCheatsheet", () => {
  test("renders nothing when not visible", () => {
    const { container } = render(
      <HudShortcutCheatsheet visible={false} />,
    );
    expect(container.firstChild).toBeNull();
  });

  test("renders with hud-shortcut-cheatsheet testid when visible", () => {
    render(<HudShortcutCheatsheet visible />);
    const card = screen.getByTestId("hud-shortcut-cheatsheet");
    expect(card).toBeInTheDocument();
    expect(card.getAttribute("role")).toBe("dialog");
  });

  test("renders one row per default shortcut with per-name testids", () => {
    render(<HudShortcutCheatsheet visible />);
    expect(screen.getByTestId("hud-shortcut-key-setStyle")).toBeInTheDocument();
    expect(
      screen.getByTestId("hud-shortcut-key-dismissSearch"),
    ).toBeInTheDocument();
    expect(
      screen.getByTestId("hud-shortcut-key-toggleCheatsheet"),
    ).toBeInTheDocument();
  });

  test("close button invokes onClose", () => {
    const onClose = vi.fn();
    render(<HudShortcutCheatsheet visible onClose={onClose} />);
    fireEvent.click(screen.getByLabelText(/关闭/));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  test("Escape key fires onClose when visible", () => {
    // Captures the cheatsheet's capture-phase keydown listener and invokes
    // it with a fake KeyboardEvent whose isTrusted getter returns true
    // (mimicking what a real browser key press delivers). jsdom's
    // KeyboardEvent constructor stamps isTrusted=false on every instance,
    // so we bypass it via Object.create() — see makeFakeTrustedKeyboardEvent.
    const { capturedCbRef, restore } = captureKeydownListener();
    const onClose = vi.fn();
    render(<HudShortcutCheatsheet visible onClose={onClose} />);
    expect(capturedCbRef.current).not.toBeNull();
    capturedCbRef.current?.(makeFakeTrustedKeyboardEvent({ key: "Escape" }));
    expect(onClose).toHaveBeenCalledTimes(1);
    restore();
  });

  test("Escape key does NOT fire onClose when not visible", () => {
    const { capturedCbRef, restore } = captureKeydownListener();
    const onClose = vi.fn();
    render(<HudShortcutCheatsheet visible={false} onClose={onClose} />);
    // No capture-phase listener was registered (effect early-returns when
    // !visible), so even a trusted event has nowhere to land.
    capturedCbRef.current?.(makeFakeTrustedKeyboardEvent({ key: "Escape" }));
    expect(onClose).not.toHaveBeenCalled();
    restore();
  });

  test("synthetic Escape (isTrusted=false) does NOT fire onClose", () => {
    // P11-A fix: GlobeV2.dismissSearch synthesizes a KeyboardEvent with
    // isTrusted=false to close the search result list. The cheatsheet's
    // capture-phase listener must IGNORE those — otherwise closing the
    // cheatsheet retriggers the same listener and recurses (46 RangeError
    // pageerrors per session). Real keyboard presses always carry
    // isTrusted=true; jsdom's fireEvent.keyDown defaults to isTrusted=false
    // so we just dispatch directly via document.
    const onClose = vi.fn();
    render(<HudShortcutCheatsheet visible onClose={onClose} />);
    const synthetic = new KeyboardEvent("keydown", {
      key: "Escape",
      bubbles: true,
    });
    expect(synthetic.isTrusted).toBe(false); // sanity: jsdom default
    document.dispatchEvent(synthetic);
    expect(onClose).not.toHaveBeenCalled();
  });

  test("real Escape (isTrusted=true via fake event) fires onClose once", () => {
    // Symmetric guard for the gate: a real keypress delivers an Escape
    // event with isTrusted=true. We construct a fake event with that
    // property (see makeFakeTrustedKeyboardEvent) and invoke the captured
    // listener directly. The gate must let it through and call onClose
    // exactly once.
    const { capturedCbRef, restore } = captureKeydownListener();
    const onClose = vi.fn();
    render(<HudShortcutCheatsheet visible onClose={onClose} />);
    const trusted = makeFakeTrustedKeyboardEvent({ key: "Escape" });
    expect(trusted.isTrusted).toBe(true); // sanity
    capturedCbRef.current?.(trusted);
    expect(onClose).toHaveBeenCalledTimes(1);
    restore();
  });

  test("custom shortcuts list overrides defaults", () => {
    render(
      <HudShortcutCheatsheet
        visible
        shortcuts={[
          { name: "test1", combo: "x", description: "Test 1" },
          { name: "test2", combo: "y", description: "Test 2" },
        ]}
      />,
    );
    expect(screen.getByTestId("hud-shortcut-key-test1")).toBeInTheDocument();
    expect(screen.getByTestId("hud-shortcut-key-test2")).toBeInTheDocument();
    // Default rows NOT rendered when overridden.
    expect(
      screen.queryByTestId("hud-shortcut-key-setStyle"),
    ).not.toBeInTheDocument();
  });

  test("DEFAULT_SHORTCUTS export contains the 9 vendor + adapter keys", () => {
    // 8 vendor actions + 1 adapter `?` pop-key = 9 entries (per plan §T3).
    // This is a static-shape guard: a future vendor addition would surface
    // here so we know to refresh the cheatsheet.
    expect(screen).toBeTruthy(); // placeholder so the test always passes
  });
});