// GEV P12 T3 — aircraft source contract + behavior suite.
//
// Explicit vitest imports (repo convention, cf. ./stubs.test.ts): vitest.config.ts
// sets globals:true for the runtime, but `tsc -b` does not load vitest/globals
// types, so bare `test`/`expect` break `npm run build`.
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ApiFetch } from "../http";
import { makeApiFetch } from "../http";
import { createIntelHubAircraftSource } from "../aircraft-source";

const jsonResponse = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status });

const abortAware = (_path: string, init?: RequestInit) =>
  new Promise<Response>((_resolve, reject) => {
    const signal = init?.signal;
    const fail = () => reject(new DOMException("aborted", "AbortError"));
    if (!signal) return;
    if (signal.aborted) return fail();
    signal.addEventListener("abort", fail);
  });

describe("createIntelHubAircraftSource constructor contract", () => {
  it("throws if apiFetch is missing", () => {
    expect(() =>
      createIntelHubAircraftSource({ apiFetch: null as unknown as ApiFetch }),
    ).toThrow(TypeError);
  });

  it("throws if apiFetch is not a function", () => {
    expect(() =>
      createIntelHubAircraftSource({ apiFetch: 42 as unknown as ApiFetch }),
    ).toThrow(TypeError);
  });

  it("label is hub.intelhub", () => {
    const src = createIntelHubAircraftSource({ apiFetch: vi.fn<ApiFetch>() });
    expect(src.label).toBe("hub.intelhub");
  });
});

describe("getTrack", () => {
  let mockFetch: ReturnType<typeof vi.fn<ApiFetch>>;
  let source: ReturnType<typeof createIntelHubAircraftSource>;

  beforeEach(() => {
    mockFetch = vi.fn<ApiFetch>();
    source = createIntelHubAircraftSource({ apiFetch: mockFetch });
  });

  it("sends /api/opensky-track with lowercase hex", async () => {
    mockFetch.mockResolvedValue(
      jsonResponse({
        path: [[1726845215, 40.69, -74.17, 10500, 91, false]],
      }),
    );
    const result = await source.getTrack("4CA9B1");
    expect(mockFetch).toHaveBeenCalledWith(
      "/api/opensky-track?icao24=4ca9b1",
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    expect(result.records).toHaveLength(1);
    expect(result.records[0]).toMatchObject({
      observedAtMs: 1726845215000,
      latitude: 40.69,
      longitude: -74.17,
      baroAltitudeM: 10500,
      courseDeg: 91,
      onGround: false,
    });
    expect(result.complete).toBe(false);
  });

  it("silent fallback on 404", async () => {
    mockFetch.mockResolvedValue(jsonResponse({}, 404));
    const result = await source.getTrack("4ca9b1");
    expect(result.records).toEqual([]);
    expect(result.complete).toBe(false);
  });

  it("silent fallback on 5xx", async () => {
    mockFetch.mockResolvedValue(jsonResponse({}, 503));
    const result = await source.getTrack("4ca9b1");
    expect(result.records).toEqual([]);
  });

  it("silent fallback on network error", async () => {
    mockFetch.mockRejectedValue(new Error("network"));
    const result = await source.getTrack("4ca9b1");
    expect(result.records).toEqual([]);
  });

  it("respects AbortSignal (composes caller signal with the 8s timeout)", async () => {
    const controller = new AbortController();
    mockFetch.mockImplementation(abortAware);
    const promise = source.getTrack("4ca9b1", { signal: controller.signal });
    const passed = mockFetch.mock.calls[0]?.[1]?.signal;
    expect(passed).toBeInstanceOf(AbortSignal);
    // AbortSignal.any() produces a fresh signal, not the caller's instance.
    expect(passed).not.toBe(controller.signal);
    controller.abort();
    await expect(promise).resolves.toEqual({ records: [], complete: false });
  });

  it("8s timeout via AbortSignal.timeout", async () => {
    const timeoutController = new AbortController();
    const timeoutSpy = vi
      .spyOn(AbortSignal, "timeout")
      .mockReturnValue(timeoutController.signal);
    mockFetch.mockImplementation(abortAware);
    const promise = source.getTrack("4ca9b1");
    expect(timeoutSpy).toHaveBeenCalledWith(8000);
    // 8s timeout fires → fetch rejects → catch returns empty.
    timeoutController.abort();
    await expect(promise).resolves.toEqual({ records: [], complete: false });
    timeoutSpy.mockRestore();
  });

  it("returns empty for non-hex reference (defensive)", async () => {
    const result = await source.getTrack("not-hex");
    expect(result.records).toEqual([]);
    expect(mockFetch).not.toHaveBeenCalled();
  });

  it("normalizes multi-waypoint response", async () => {
    mockFetch.mockResolvedValue(
      jsonResponse({
        path: [
          [1726845215, 40.69, -74.17, 10500, 91, false],
          [1726845300, 40.7, -74.18, 10600, 92, false],
        ],
      }),
    );
    const result = await source.getTrack("4ca9b1");
    expect(result.records).toHaveLength(2);
    expect(result.records[1].observedAtMs).toBe(1726845300000);
  });

  it("skips malformed waypoints", async () => {
    mockFetch.mockResolvedValue(
      jsonResponse({
        path: [
          [1726845215, 40.69, -74.17, 10500, 91, false],
          [null, null, null], // invalid time/coords
          "not-array", // invalid row
          [1726845500, 40.71], // too short (no longitude)
        ],
      }),
    );
    const result = await source.getTrack("4ca9b1");
    expect(result.records).toHaveLength(1);
    expect(result.records[0].observedAtMs).toBe(1726845215000);
  });
});

describe("getEnrichment", () => {
  let mockFetch: ReturnType<typeof vi.fn<ApiFetch>>;
  let source: ReturnType<typeof createIntelHubAircraftSource>;

  beforeEach(() => {
    mockFetch = vi.fn<ApiFetch>();
    source = createIntelHubAircraftSource({ apiFetch: mockFetch });
  });

  it("type path: lowercase hex, passes through found:true", async () => {
    mockFetch.mockResolvedValue(
      jsonResponse({ found: true, typeCode: "B738", typeName: "Boeing 737-800" }),
    );
    const r = await source.getEnrichment({ kind: "type", id: "4CA9B1" });
    expect(mockFetch).toHaveBeenCalledWith(
      "/api/adsbdb/type/4ca9b1",
      expect.any(Object),
    );
    expect(r).toEqual({
      found: true,
      typeCode: "B738",
      typeName: "Boeing 737-800",
    });
  });

  it("route path: uppercase callsign, passes through found:true", async () => {
    mockFetch.mockResolvedValue(
      jsonResponse({
        found: true,
        airline: "UA",
        origin: { code: "KEWR" },
        destination: { code: "KSFO" },
      }),
    );
    const r = await source.getEnrichment({ kind: "route", id: "ual123" });
    expect(mockFetch).toHaveBeenCalledWith(
      "/api/adsbdb/route/UAL123",
      expect.any(Object),
    );
    expect(r).toMatchObject({ found: true, airline: "UA" });
  });

  it("passes through found:false (negative result)", async () => {
    mockFetch.mockResolvedValue(jsonResponse({ found: false }));
    const r = await source.getEnrichment({ kind: "type", id: "4ca9b1" });
    expect(r).toEqual({ found: false });
  });

  it("throws on 5xx (vendor records cooldown)", async () => {
    mockFetch.mockResolvedValue(jsonResponse({}, 503));
    await expect(
      source.getEnrichment({ kind: "type", id: "4ca9b1" }),
    ).rejects.toThrow();
  });

  it("throws on invalid hex (type)", async () => {
    await expect(
      source.getEnrichment({ kind: "type", id: "xyz" }),
    ).rejects.toThrow(/6-char hex/);
    expect(mockFetch).not.toHaveBeenCalled();
  });

  it("throws on invalid callsign (route)", async () => {
    await expect(
      source.getEnrichment({ kind: "route", id: "x" }),
    ).rejects.toThrow(/2-8 char/);
  });

  it("throws on unsupported kind", async () => {
    await expect(
      source.getEnrichment({ kind: "weather" as "type", id: "x" }),
    ).rejects.toThrow(/unsupported/);
  });
});

describe("auth reachability (T1 review Important 3)", () => {
  it("getEnrichment carries the hub Authorization header through makeApiFetch", async () => {
    const globalFetch = vi.fn();
    vi.stubGlobal("fetch", globalFetch);
    globalFetch.mockResolvedValue(jsonResponse({ found: false }));
    try {
      const source = createIntelHubAircraftSource({
        apiFetch: makeApiFetch("http://hub.test", "ihk_test_key"),
      });
      await source.getEnrichment({ kind: "type", id: "4CA9B1" });
      expect(globalFetch).toHaveBeenCalledWith(
        "http://hub.test/api/adsbdb/type/4ca9b1",
        expect.objectContaining({
          headers: { Authorization: "Bearer ihk_test_key" },
        }),
      );
    } finally {
      vi.unstubAllGlobals();
    }
  });
});
