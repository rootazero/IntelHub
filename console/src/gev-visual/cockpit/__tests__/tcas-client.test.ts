import { describe, expect, test, vi } from "vitest";
import {
  mountCockpitTcas,
  type TcasSnapshot,
} from "../tcas-client";

function fakeSnapshot(): TcasSnapshot {
  return {
    ts: "2026-09-23T00:00:00Z",
    query: { lat: 25.45, lng: 51.63, radius_nm: 5 },
    count: 2,
    aircraft: [
      {
        hex: "abc",
        flight: "MSR935",
        lat: 25.46,
        lon: 51.64,
        alt_m: 3500,
        gs: 360,
        track: 180,
        squawk: null,
        mil: false,
        distance_nm: 0.5,
        bearing_deg: 10,
        closure_kts: 350,
        threat: "warning",
        age_s: 1.0,
      },
      {
        hex: "def",
        flight: "QTR1171",
        lat: 25.50,
        lon: 51.70,
        alt_m: 10000,
        gs: 200,
        track: 90,
        squawk: null,
        mil: false,
        distance_nm: 4.5,
        bearing_deg: 60,
        closure_kts: 50,
        threat: "monitor",
        age_s: 1.0,
      },
    ],
  };
}

describe("mountCockpitTcas (GEV P20)", () => {
  test("mount returns a handle with start / stop / destroy", () => {
    const handle = mountCockpitTcas({
      authToken: "test-token",
      baseUrl: "http://example",
    });
    expect(typeof handle.start).toBe("function");
    expect(typeof handle.stop).toBe("function");
    expect(typeof handle.destroy).toBe("function");
    handle.destroy();
  });

  test("start fires fetch immediately and pipes snapshot to callback", async () => {
    const fetchSpy = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue({
        ok: true,
        json: async () => fakeSnapshot(),
      } as Response);
    const handle = mountCockpitTcas({
      authToken: "test-token",
      baseUrl: "http://example",
      pollMs: 100,
    });
    const received: TcasSnapshot[] = [];
    handle.start(
      (s) => received.push(s),
      () => ({ lat: 25.45, lng: 51.63 }),
    );
    // Wait a tick for the immediate fetch + microtask resolution.
    await new Promise((r) => setTimeout(r, 20));
    expect(received.length).toBeGreaterThanOrEqual(1);
    expect(received[0].count).toBe(2);
    handle.destroy();
    fetchSpy.mockRestore();
  });

  test("fetch URL includes lat, lng, radius_nm", async () => {
    const fetchSpy = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue({
        ok: true,
        json: async () => fakeSnapshot(),
      } as Response);
    const handle = mountCockpitTcas({
      authToken: "tok",
      baseUrl: "http://hub",
      pollMs: 100,
      radiusNm: 7,
    });
    handle.start(() => undefined, () => ({ lat: 1.0, lng: 2.0 }));
    await new Promise((r) => setTimeout(r, 20));
    expect(fetchSpy).toHaveBeenCalled();
    const url = (fetchSpy.mock.calls[0]?.[0] as string) ?? "";
    expect(url).toContain("lat=1");
    expect(url).toContain("lng=2");
    expect(url).toContain("radius_nm=7");
    handle.destroy();
    fetchSpy.mockRestore();
  });

  test("fetch sends Bearer auth header", async () => {
    const fetchSpy = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue({
        ok: true,
        json: async () => fakeSnapshot(),
      } as Response);
    const handle = mountCockpitTcas({
      authToken: "my-token",
      baseUrl: "http://hub",
      pollMs: 100,
    });
    handle.start(() => undefined, () => ({ lat: 0, lng: 0 }));
    await new Promise((r) => setTimeout(r, 20));
    const init = fetchSpy.mock.calls[0]?.[1] as RequestInit | undefined;
    const headers = (init?.headers ?? {}) as Record<string, string>;
    expect(headers.Authorization).toBe("Bearer my-token");
    handle.destroy();
    fetchSpy.mockRestore();
  });

  test("fetch error returns null and does not crash", async () => {
    const fetchSpy = vi
      .spyOn(globalThis, "fetch")
      .mockRejectedValue(new Error("network down"));
    const handle = mountCockpitTcas({
      authToken: "tok",
      baseUrl: "http://hub",
      pollMs: 100,
    });
    const received: TcasSnapshot[] = [];
    expect(() =>
      handle.start(
        (s) => received.push(s),
        () => ({ lat: 25, lng: 51 }),
      ),
    ).not.toThrow();
    await new Promise((r) => setTimeout(r, 30));
    expect(received.length).toBe(0);
    handle.destroy();
    fetchSpy.mockRestore();
  });

  test("non-ok response returns null", async () => {
    const fetchSpy = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue({
        ok: false,
        status: 503,
        json: async () => ({}),
      } as Response);
    const handle = mountCockpitTcas({
      authToken: "tok",
      baseUrl: "http://hub",
      pollMs: 100,
    });
    const received: TcasSnapshot[] = [];
    handle.start(
      (s) => received.push(s),
      () => ({ lat: 25, lng: 51 }),
    );
    await new Promise((r) => setTimeout(r, 20));
    expect(received.length).toBe(0);
    handle.destroy();
    fetchSpy.mockRestore();
  });

  test("stop() clears interval; no further snapshots", async () => {
    const fetchSpy = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue({
        ok: true,
        json: async () => fakeSnapshot(),
      } as Response);
    const handle = mountCockpitTcas({
      authToken: "tok",
      baseUrl: "http://hub",
      pollMs: 50,
    });
    const received: TcasSnapshot[] = [];
    handle.start(
      (s) => received.push(s),
      () => ({ lat: 0, lng: 0 }),
    );
    await new Promise((r) => setTimeout(r, 80));
    const before = received.length;
    handle.stop();
    await new Promise((r) => setTimeout(r, 80));
    expect(received.length).toBe(before);
    handle.destroy();
    fetchSpy.mockRestore();
  });

  test("destroy() makes subsequent fetches a no-op", async () => {
    const fetchSpy = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue({
        ok: true,
        json: async () => fakeSnapshot(),
      } as Response);
    const handle = mountCockpitTcas({
      authToken: "tok",
      baseUrl: "http://hub",
      pollMs: 50,
    });
    handle.start(() => undefined, () => ({ lat: 0, lng: 0 }));
    handle.destroy();
    await new Promise((r) => setTimeout(r, 100));
    // Only the immediate fetch fired before destroy (max 1); no further.
    expect(fetchSpy.mock.calls.length).toBeLessThanOrEqual(1);
    fetchSpy.mockRestore();
  });

  test("null getAgentLatLng result skips the tick", async () => {
    const fetchSpy = vi.spyOn(globalThis, "fetch");
    const handle = mountCockpitTcas({
      authToken: "tok",
      baseUrl: "http://hub",
      pollMs: 50,
    });
    handle.start(() => undefined, () => null);
    await new Promise((r) => setTimeout(r, 100));
    expect(fetchSpy).not.toHaveBeenCalled();
    handle.destroy();
    fetchSpy.mockRestore();
  });
});