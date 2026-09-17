// Explicit vitest imports (repo convention, cf. ./vessels.test.ts).
import { expect, test, vi } from "vitest";
import type { ApiFetch } from "../http";
import { cctvSource, frameUrlFor, mediaUrlFor } from "../cctv";
import { createIntelHubLayerSources } from "../index";

const jsonResponse = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status });

const CAM = {
  id: "tfl:JamCams_00002.00865",
  name: "A406 Billet Upass E",
  city: "London",
  lat: 51.60067,
  lon: -0.01594,
  headingDeg: 270.4,
  fovDeg: 74,
  pitchDeg: -17,
};

test("getCatalog reads the hub route and passes the sources array through", async () => {
  const calls: string[] = [];
  const apiFetch: ApiFetch = async (path) => {
    calls.push(String(path));
    return jsonResponse({ sources: [{ id: "tfl:x", lat: 1, lon: 2 }] });
  };
  const out = await cctvSource(apiFetch).getCatalog();
  expect(calls[0]).toBe("/api/v1/gev/cctv/sources");
  expect(out.sources).toHaveLength(1);
});

test("getCatalog/getHealth reject malformed envelopes (engine error text)", async () => {
  const bad: ApiFetch = async () => jsonResponse({ sources: "nope" });
  await expect(cctvSource(bad).getCatalog()).rejects.toThrow("Malformed camera sources snapshot");
  const bad2: ApiFetch = async () => jsonResponse({ cameras: null });
  await expect(cctvSource(bad2).getHealth()).rejects.toThrow("Malformed camera cameras snapshot");
  const s503: ApiFetch = async () => jsonResponse({}, 503);
  await expect(cctvSource(s503).getCatalog()).rejects.toThrow("Camera source HTTP 503");
});

test("getFrameUrl builds the hub proxy URL with the vendor query contract", () => {
  const url = frameUrlFor(CAM, 10000);
  expect(url.startsWith("/api/v1/gev/cctv/frame/tfl%3AJamCams_00002.00865?")).toBe(true);
  const q = new URLSearchParams(url.split("?")[1]);
  expect(q.get("label")).toBe("A406 Billet Upass E");
  expect(q.get("city")).toBe("London");
  expect(q.get("lat")).toBe("51.600670");
  expect(q.get("lon")).toBe("-0.015940");
  expect(q.get("heading")).toBe("270"); // rounded
  expect(q.get("fov")).toBe("74");
  expect(q.get("pitch")).toBe("-17");
  expect(Number(q.get("ts"))).toBeGreaterThan(0);
});

test("getFrameUrl tolerates sparse camera records (defaults engage)", () => {
  const url = frameUrlFor({ id: "on:42" });
  expect(url).toContain("/api/v1/gev/cctv/frame/on%3A42?");
  const q = new URLSearchParams(url.split("?")[1]);
  expect(q.get("fov")).toBe("74"); // vendor default
  expect(q.get("pitch")).toBe("-10"); // frameUrlFor default (source.js)
});

test("getMediaUrl builds the hub proxy URL on a 15s grid", () => {
  const url = mediaUrlFor(CAM);
  expect(url.startsWith("/api/v1/gev/cctv/media/tfl%3AJamCams_00002.00865?ts=")).toBe(true);
});

test("factory wires the real cctv source (stub replaced)", () => {
  const s = createIntelHubLayerSources({ apiFetch: async () => jsonResponse({ sources: [] }) });
  const c = s.cctv as Record<string, unknown>;
  for (const m of ["getCatalog", "getHealth", "getFrameUrl", "getMediaUrl"])
    expect(typeof c[m]).toBe("function");
  // and it's the real one: getFrameUrl returns a hub proxy URL, not ""
  const url = (c.getFrameUrl as (cam: { id: string }) => string)({ id: "x" });
  expect(url).toContain("/api/v1/gev/cctv/frame/x");
});

test("abort signal is honored", async () => {
  const apiFetch: ApiFetch = vi.fn(async () => jsonResponse({ sources: [] }));
  const src = cctvSource(apiFetch);
  const ctl = new AbortController();
  ctl.abort();
  await expect(src.getCatalog({ signal: ctl.signal })).rejects.toThrow();
  expect(apiFetch).not.toHaveBeenCalled();
});
