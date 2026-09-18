// GEV P8 — HudDrawToolbar component tests.
//
// Mock boundary: the draw-tool adapter (src/gev-visual/annotations/draw-tool)
// is mocked because its job — the seam over the pure vendor drawMode.js — is
// already unit-tested in gev-visual/__tests__/draw-tool.test.ts. The mock
// REPLICATES the real handle contract faithfully (start/addClickWorld with a
// finite-coordinate session, finish → spec | null, onPreview subscription,
// destroy idempotence) so the component wiring is what's under test, not a
// lenient fake. The annotation-store stays REAL (pure fetch wrapper) so the
// finish path asserts the actual `POST /api/v1/annotations` shape through the
// injected apiFetch.
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import "@testing-library/jest-dom/vitest";
import { HudDrawToolbar } from "../HudDrawToolbar";

const hoisted = vi.hoisted(() => ({
  mountCalls: 0,
  addClickCalls: [] as Array<{ lon: number; lat: number }>,
}));

// Faithful draw-tool adapter mock. Session + preview subscriptions live inside
// the mock factory so they reset via destroy() on every unmount (afterEach
// cleanup), mirroring the real adapter's lifecycle.
vi.mock("../../gev-visual/annotations/draw-tool", () => {
  type Vertex = { lon: number; lat: number };
  const previewSubs = new Set<(vertices: Array<Vertex>) => void>();
  let session: { shape: "pin" | "line" | "area"; vertices: Array<Vertex> } | null =
    null;
  const handle = {
    start(mode: "pin" | "line" | "area") {
      session = { shape: mode, vertices: [] };
    },
    addClickWorld(lon: number, lat: number) {
      if (!session) return false;
      if (!Number.isFinite(lon) || !Number.isFinite(lat)) return false;
      session.vertices.push({ lon, lat });
      hoisted.addClickCalls.push({ lon, lat });
      previewSubs.forEach((cb) => cb(session!.vertices));
      return true;
    },
    finish(opts?: { label?: string; persist?: boolean }) {
      if (!session) return null;
      const spec = {
        id: "spec-local-1",
        shape: session.shape,
        vertices: [...session.vertices],
        label: opts?.label ?? undefined,
        color: "primary" as const,
      };
      session = null;
      return spec;
    },
    cancel() {
      session = null;
    },
    onPreview(cb: (vertices: Array<Vertex>) => void) {
      previewSubs.add(cb);
      return () => {
        previewSubs.delete(cb);
      };
    },
    onState() {
      return () => {};
    },
    destroy() {
      session = null;
      previewSubs.clear();
    },
  };
  return {
    mountDrawTool: () => {
      hoisted.mountCalls += 1;
      return handle;
    },
  };
});

const mockViewer = {
  scene: { pickPosition: () => ({ x: 0, y: 0, z: 0 }) },
};

const mockApiFetch = vi.fn(
  () => Promise.resolve(new Response("{}", { status: 200 })),
);

const serverRow = (overrides: Record<string, unknown> = {}) => ({
  id: "saved-1",
  agent_id: null,
  shape: "pin",
  label: "Target",
  color: "primary",
  geometry: { vertices: [{ lon: 1, lat: 2 }] },
  ttl_ms: null,
  meta: {},
  created_at: "2026-09-18T00:00:00Z",
  expires_at: null,
  ...overrides,
});

beforeEach(() => {
  hoisted.mountCalls = 0;
  hoisted.addClickCalls = [];
  mockApiFetch.mockReset();
  mockApiFetch.mockImplementation(() =>
    Promise.resolve(new Response("{}", { status: 200 })),
  );
  // The vendored engine installs this at module scope (annotationEngine.js).
  // The toolbar only needs the Cartesian3→Cartographic conversion + toDegrees.
  (window as unknown as Record<string, unknown>).__CESIUM__ = {
    Cartographic: { fromCartesian: () => ({ longitude: 1, latitude: 2 }) },
    Math: { toDegrees: (r: number) => r },
  };
});

afterEach(cleanup);

describe("HudDrawToolbar", () => {
  test("not rendered when visible=false", () => {
    const { container } = render(
      <HudDrawToolbar
        viewer={mockViewer}
        apiFetch={mockApiFetch}
        visible={false}
        onClose={() => {}}
      />,
    );
    expect(
      container.querySelector('[data-testid="hud-draw-toolbar"]'),
    ).toBeNull();
  });

  test("renders three mode buttons + cancel/finish when visible=true", () => {
    render(
      <HudDrawToolbar
        viewer={mockViewer}
        apiFetch={mockApiFetch}
        visible
        onClose={() => {}}
      />,
    );
    expect(screen.getByTestId("hud-draw-mode-pin")).toBeTruthy();
    expect(screen.getByTestId("hud-draw-mode-line")).toBeTruthy();
    expect(screen.getByTestId("hud-draw-mode-area")).toBeTruthy();
    expect(screen.getByTestId("hud-draw-cancel")).toBeTruthy();
    expect(screen.getByTestId("hud-draw-finish")).toBeTruthy();
  });

  test("pin mode: 1 click → finish enabled; click finish → POST /api/v1/annotations", async () => {
    const onCreated = vi.fn();
    mockApiFetch.mockResolvedValueOnce(
      new Response(JSON.stringify(serverRow()), { status: 200 }),
    );
    render(
      <HudDrawToolbar
        viewer={mockViewer}
        apiFetch={mockApiFetch}
        visible
        onClose={() => {}}
        onCreated={onCreated}
      />,
    );
    // Wait for the lazy adapter mount (import + onPreview wiring).
    await waitFor(() => expect(hoisted.mountCalls).toBe(1));

    fireEvent.click(screen.getByTestId("hud-draw-mode-pin"));
    // Simulate a canvas click: pickPosition returns a Cartesian3-like {x,y,z},
    // __CESIUM__ converts it to (lon 1, lat 2) via toDegrees identity.
    fireEvent.click(screen.getByTestId("hud-draw-toolbar"), {
      clientX: 100,
      clientY: 50,
    });
    expect(hoisted.addClickCalls).toEqual([{ lon: 1, lat: 2 }]);
    expect(screen.getByTestId("hud-draw-finish")).toBeEnabled();

    fireEvent.click(screen.getByTestId("hud-draw-finish"));
    await waitFor(() => expect(onCreated).toHaveBeenCalledTimes(1));
    // Real annotation-store POSTs to the REST endpoint through apiFetch.
    expect(mockApiFetch).toHaveBeenCalledWith(
      "/api/v1/annotations",
      expect.objectContaining({ method: "POST" }),
    );
    // The saved row (server id) flows back through onCreated.
    expect(onCreated).toHaveBeenCalledWith(
      expect.objectContaining({ id: "saved-1", shape: "pin" }),
    );
  });

  test("cancel button resets state and calls onClose", () => {
    const onClose = vi.fn();
    render(
      <HudDrawToolbar
        viewer={mockViewer}
        apiFetch={mockApiFetch}
        visible
        onClose={onClose}
      />,
    );
    fireEvent.click(screen.getByTestId("hud-draw-cancel"));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  test("label input updates label preview", () => {
    render(
      <HudDrawToolbar
        viewer={mockViewer}
        apiFetch={mockApiFetch}
        visible
        onClose={() => {}}
      />,
    );
    const label = screen.getByTestId("hud-draw-label") as HTMLInputElement;
    fireEvent.change(label, { target: { value: "Target Zone" } });
    expect(label.value).toBe("Target Zone");
  });
});
