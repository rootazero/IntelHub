// GEV P10 T2 — scene-sharing adapter tests.
//
// Both vendor exports are pure DOM factories; tests run against real jsdom
// DOM and verify the adapter wires edit/share callbacks and the disposer.
//
// jsdom lacks HTMLDialogElement.showModal (added in Chrome 37+ but only in
// the optional dialog-polyfill set). The vendor calls showModal() on
// construction, so we polyfill it on the prototype once for the test file.
import { afterEach, beforeAll, describe, expect, test, vi } from "vitest";
import {
  createSceneDialog,
  mountSceneSharing,
} from "../scene-sharing";

beforeAll(() => {
  const proto = HTMLDialogElement.prototype as HTMLDialogElement & {
    showModal?: () => void;
  };
  if (typeof proto.showModal !== "function") {
    Object.defineProperty(proto, "showModal", {
      value: function (this: HTMLDialogElement) {
        this.setAttribute("open", "");
      },
      writable: true,
    });
  }
  if (typeof (proto as HTMLDialogElement & { close?: () => void }).close !== "function") {
    Object.defineProperty(proto, "close", {
      value: function (this: HTMLDialogElement) {
        this.removeAttribute("open");
      },
      writable: true,
    });
  }
});

describe("scene-sharing adapter — createSceneDialog", () => {
  afterEach(() => {
    document.body.innerHTML = "";
    vi.restoreAllMocks();
  });

  test("creates a <dialog> element appended to document.body", () => {
    const onClose = vi.fn();
    const handle = createSceneDialog("Test dialog", onClose);
    expect(handle.element.tagName).toBe("DIALOG");
    expect(document.body.contains(handle.element)).toBe(true);
    handle.dispose();
  });

  test("dispose() removes the dialog from document.body", () => {
    const handle = createSceneDialog("disposable", () => {});
    handle.dispose();
    expect(document.body.contains(handle.element)).toBe(false);
  });

  test("text() appends a <p> with the given text", () => {
    const handle = createSceneDialog("text test", () => {});
    const p = handle.text("Hello world");
    expect(p.tagName).toBe("P");
    expect(p.textContent).toBe("Hello world");
    handle.dispose();
  });

  test("button() appends a <button> and binds click handler", () => {
    const handle = createSceneDialog("button test", () => {});
    const onClick = vi.fn();
    const btn = handle.button("Save", onClick);
    expect(btn.tagName).toBe("BUTTON");
    expect(btn.textContent).toBe("Save");
    btn.click();
    expect(onClick).toHaveBeenCalledTimes(1);
    handle.dispose();
  });

  test("input() creates an input element", () => {
    const handle = createSceneDialog("input test", () => {});
    const node = handle.input("Name", "initial");
    // Vendor returns the input/textarea element directly (the wrapper <label>
    // is appended to the body but not returned). tagName is INPUT for default
    // {multiline:false} and TEXTAREA for {multiline:true}.
    expect(["INPUT", "TEXTAREA"]).toContain(node.tagName);
    handle.dispose();
  });

  test("Cancel button triggers onClose", () => {
    const onClose = vi.fn();
    const handle = createSceneDialog("cancel test", onClose);
    // Vendor always appends a Cancel button as the first footer child.
    const cancelBtn = handle.footer.querySelector("button");
    cancelBtn?.click();
    expect(onClose).toHaveBeenCalled();
    handle.dispose();
  });
});

describe("scene-sharing adapter — mountSceneSharing", () => {
  afterEach(() => {
    document.body.innerHTML = "";
    vi.restoreAllMocks();
  });

  test("appends a button bar to the supplied panel", () => {
    const panel = document.createElement("div");
    panel.id = "scene-panel";
    document.body.appendChild(panel);
    const handle = mountSceneSharing(panel, {
      edit: () => {},
      share: () => {},
    });
    const bar = panel.querySelector('[data-director-authoring]');
    expect(bar).not.toBeNull();
    handle.destroy();
  });

  test("edit button triggers the edit callback", () => {
    const panel = document.createElement("div");
    document.body.appendChild(panel);
    const edit = vi.fn();
    const share = vi.fn();
    const handle = mountSceneSharing(panel, { edit, share });
    const buttons = panel.querySelectorAll("button");
    buttons[0]?.click();
    expect(edit).toHaveBeenCalled();
    handle.destroy();
  });

  test("share button triggers the share callback", () => {
    const panel = document.createElement("div");
    document.body.appendChild(panel);
    const edit = vi.fn();
    const share = vi.fn();
    const handle = mountSceneSharing(panel, { edit, share });
    const buttons = panel.querySelectorAll("button");
    buttons[1]?.click();
    expect(share).toHaveBeenCalled();
    handle.destroy();
  });

  test("destroy() removes the button bar", () => {
    const panel = document.createElement("div");
    document.body.appendChild(panel);
    const handle = mountSceneSharing(panel, { edit: () => {}, share: () => {} });
    handle.destroy();
    expect(panel.querySelector('[data-director-authoring]')).toBeNull();
  });

  test("destroy() is idempotent", () => {
    const panel = document.createElement("div");
    document.body.appendChild(panel);
    const handle = mountSceneSharing(panel, { edit: () => {}, share: () => {} });
    handle.destroy();
    expect(() => handle.destroy()).not.toThrow();
  });
});