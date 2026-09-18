import { describe, expect, test, vi } from "vitest";
import { createAnnotationStore } from "../annotations/annotation-store";
import type { AnnotationSpec } from "../annotations/draw-tool";

// Wire row shape mirroring hub-core `db::annotations::AnnotationRow` (Task 2):
// `geometry` is `{vertices:[{lon,lat,height?}]}`, `label`/`ttl_ms` are nullable.
interface ServerRow {
  id: string;
  agent_id: string | null;
  shape: string;
  label: string | null;
  color: string;
  geometry: { vertices: Array<{ lon: number; lat: number; height?: number }> };
  ttl_ms: number | null;
  meta: Record<string, unknown>;
  created_at: string;
  expires_at: string | null;
}

function makeRow(overrides: Partial<ServerRow> = {}): ServerRow {
  return {
    id: "11111111-2222-3333-4444-555555555555",
    agent_id: "pi",
    shape: "pin",
    label: "Mark",
    color: "primary",
    geometry: { vertices: [{ lon: 12.3, lat: 45.6, height: 0 }] },
    ttl_ms: null,
    meta: {},
    created_at: "2026-09-18T00:00:00Z",
    expires_at: null,
    ...overrides,
  };
}

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function makeSpec(overrides: Partial<AnnotationSpec> = {}): AnnotationSpec {
  return {
    id: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
    shape: "area",
    vertices: [
      { lon: 0, lat: 0 },
      { lon: 1, lat: 0 },
      { lon: 0, lat: 1 },
    ],
    color: "amber",
    ...overrides,
  };
}

function recordCall(fetchFn: ReturnType<typeof vi.fn>) {
  const call = fetchFn.mock.calls[fetchFn.mock.calls.length - 1];
  return { url: call[0] as string, init: (call[1] ?? {}) as RequestInit };
}

describe("createAnnotationStore", () => {
  test("list uses since query param and maps rows to specs", async () => {
    const fetchFn = vi.fn(async () => jsonResponse({ annotations: [makeRow()] }));
    const store = createAnnotationStore(fetchFn);
    const specs = await store.list({ since: "-30m" });
    const { url } = recordCall(fetchFn);
    expect(url).toBe("/api/v1/annotations?since=-30m");
    expect(specs).toHaveLength(1);
    expect(specs[0].id).toBe("11111111-2222-3333-4444-555555555555");
    expect(specs[0].vertices).toEqual([{ lon: 12.3, lat: 45.6, height: 0 }]);
  });

  test("list joins bbox as comma-separated south,west,north,east", async () => {
    const fetchFn = vi.fn(async () => jsonResponse({ annotations: [] }));
    const store = createAnnotationStore(fetchFn);
    await store.list({ bbox: [-10, 20, 30, 40] });
    const { url } = recordCall(fetchFn);
    expect(url).toBe("/api/v1/annotations?bbox=-10%2C20%2C30%2C40");
  });

  test("create posts shape + nested geometry.vertices", async () => {
    const row = makeRow({
      id: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
      shape: "area",
      geometry: {
        vertices: [
          { lon: 0, lat: 0 },
          { lon: 1, lat: 0 },
          { lon: 0, lat: 1 },
        ],
      },
    });
    const fetchFn = vi.fn(async () => jsonResponse(row, 201));
    const store = createAnnotationStore(fetchFn);
    const spec = makeSpec();
    const result = await store.create(spec);
    const { url, init } = recordCall(fetchFn);
    expect(url).toBe("/api/v1/annotations");
    expect(init.method).toBe("POST");
    const body = JSON.parse(init.body as string);
    expect(body.shape).toBe("area");
    expect(body.geometry).toEqual({
      vertices: [
        { lon: 0, lat: 0 },
        { lon: 1, lat: 0 },
        { lon: 0, lat: 1 },
      ],
    });
    expect(result.id).toBe("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee");
  });

  test("patch label sends the new label", async () => {
    const fetchFn = vi.fn(async () => jsonResponse(makeRow({ label: "Renamed" })));
    const store = createAnnotationStore(fetchFn);
    const result = await store.patch("11111111-2222-3333-4444-555555555555", { label: "Renamed" });
    const { url, init } = recordCall(fetchFn);
    expect(url).toBe("/api/v1/annotations/11111111-2222-3333-4444-555555555555");
    expect(init.method).toBe("PATCH");
    expect(JSON.parse(init.body as string)).toEqual({ label: "Renamed" });
    expect(result.label).toBe("Renamed");
  });

  test("patch clear label sends explicit null (not absent)", async () => {
    const fetchFn = vi.fn(async () => jsonResponse(makeRow({ label: null })));
    const store = createAnnotationStore(fetchFn);
    await store.patch("11111111-2222-3333-4444-555555555555", { label: null });
    const { init } = recordCall(fetchFn);
    // The hub's double-Option PATCH reads `"label":null` as "clear the label"
    // (vs an absent key = "leave untouched"). The literal must be present.
    expect(init.body as string).toContain('"label":null');
  });

  test("remove issues DELETE and tolerates a 204 no-body", async () => {
    const fetchFn = vi.fn(async () => new Response(null, { status: 204 }));
    const store = createAnnotationStore(fetchFn);
    await expect(
      store.remove("11111111-2222-3333-4444-555555555555"),
    ).resolves.toBeUndefined();
    const { url, init } = recordCall(fetchFn);
    expect(url).toBe("/api/v1/annotations/11111111-2222-3333-4444-555555555555");
    expect(init.method).toBe("DELETE");
  });

  test("propagates non-2xx as errors for every verb", async () => {
    const fetchFn = vi.fn(async () => new Response("boom", { status: 500 }));
    const store = createAnnotationStore(fetchFn);
    await expect(store.list()).rejects.toThrow("list failed: 500");
    await expect(store.create(makeSpec())).rejects.toThrow("create failed: 500");
    await expect(store.get("x")).rejects.toThrow("get failed: 500");
    await expect(store.patch("x", { label: "y" })).rejects.toThrow("patch failed: 500");
    await expect(store.remove("x")).rejects.toThrow("remove failed: 500");
  });
});
