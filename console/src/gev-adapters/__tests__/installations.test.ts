// Explicit vitest imports (repo convention, cf. ./vessels.test.ts).
import { expect, test, vi } from "vitest";
import type { ApiFetch } from "../http";
import { createIntelHubLayerSources } from "../index";
import { installationsSource } from "../installations";

const jsonResponse = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status });

// A hub `/api/v1/gev/installations` envelope (hub-core gev_installations.rs):
// Overpass-shaped elements + the contract §4 envelope fields.
const ENVELOPE = {
  elements: [
    {
      type: "node",
      id: 123,
      lat: 48.37,
      lon: -124.9,
      tags: { military: "naval_base", name: "CFB Esquimalt" },
    },
  ],
  retrievedAt: "2026-09-17T00:00:00Z",
  status: "ok",
  saturated: false,
  elementCap: 2000,
};

const BOX = { south: 48.0, west: -125.5, north: 49.0, east: -124.0 };

test("getMappedSites rewrites the engine path to the hub route, preserving the query string", async () => {
  const calls: string[] = [];
  const apiFetch: ApiFetch = async (path, init) => {
    calls.push(String(path));
    return jsonResponse(ENVELOPE);
  };
  const src = installationsSource(apiFetch);
  const out = await src.getMappedSites(BOX);
  expect(calls).toHaveLength(1);
  expect(calls[0]).toMatch(/^\/api\/v1\/gev\/installations\?/);
  expect(calls[0]).not.toContain("/api/military-installations");
  // engine-built query params survive untouched (toFixed(5) contract)
  expect(calls[0]).toContain("south=48.00000");
  expect(calls[0]).toContain("west=-125.50000");
  // normalize ran client-side: records carry the recomposed element
  expect(out.records).toHaveLength(1);
  expect(out.saturated).toBe(false);
  expect(out.status).toBe("ok");
});

test("getMappedSites propagates the engine's own bbox TypeError (no fetch issued)", async () => {
  const apiFetch: ApiFetch = vi.fn(async () => jsonResponse(ENVELOPE));
  const src = installationsSource(apiFetch);
  await expect(
    src.getMappedSites({ south: 10, west: 0, north: 5, east: 1 }),
  ).rejects.toThrow(TypeError);
  expect(apiFetch).not.toHaveBeenCalled();
});

test("getMappedSites surfaces hub error envelope as failureReason", async () => {
  const apiFetch: ApiFetch = async () =>
    jsonResponse({ error: "installations catalog unavailable", reason: "unavailable" }, 503);
  const src = installationsSource(apiFetch);
  await expect(src.getMappedSites(BOX)).rejects.toThrow(/503|unavailable/i);
});

test("exact=1 flag is forwarded through the rewrite (saturation re-ask contract)", async () => {
  const calls: string[] = [];
  const apiFetch: ApiFetch = async (path) => {
    calls.push(String(path));
    return jsonResponse(ENVELOPE);
  };
  await installationsSource(apiFetch).getMappedSites(BOX, { exact: true });
  expect(calls[0]).toContain("exact=1");
});

test("searchNearby degrades silently to empty places (P3: no Google Places proxy)", async () => {
  const apiFetch: ApiFetch = vi.fn(async () => jsonResponse({}));
  const src = installationsSource(apiFetch);
  const out = await src.searchNearby({ latitude: 48, longitude: -124, radiusM: 5000 });
  expect(out).toEqual({ places: [] });
  expect(apiFetch).not.toHaveBeenCalled();
});

test("searchNearby mirrors the engine's TypeError validation", async () => {
  const src = installationsSource(async () => jsonResponse({}));
  await expect(
    src.searchNearby({ latitude: 48, longitude: -124, radiusM: 500 }),
  ).rejects.toThrow(TypeError);
  await expect(
    src.searchNearby({ latitude: 91, longitude: -124, radiusM: 5000 }),
  ).rejects.toThrow(TypeError);
});

test("factory wires the real installations source (stub replaced)", async () => {
  const s = createIntelHubLayerSources({ apiFetch: async () => jsonResponse(ENVELOPE) });
  const inst = s.installations as Record<string, unknown>;
  expect(typeof inst.getMappedSites).toBe("function");
  expect(typeof inst.searchNearby).toBe("function");
});
