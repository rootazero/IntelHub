// Explicit vitest imports (repo convention, cf. ./stubs.test.ts): vitest.config.ts
// sets globals:true for the runtime, but `tsc -b` does not load vitest/globals
// types, so bare `test`/`expect` break `npm run build`.
import { expect, test, vi } from "vitest";
import type { ApiFetch } from "../http";
import { createIntelHubLayerSources } from "../index";
import { satellitesSource } from "../satellites";

const TLE = "ISS (ZARYA)\n1 25544U 98067A   26260.51785714  .00012345  00000-0  23456-3 0  9992\n2 25544  51.6400 208.9163 0006317  69.9862  25.2906 15.49560532420999\n";

const textResponse = (body: string, status = 200) =>
  new Response(body, { status });

test("200 returns { ok, status, text } with the TLE body", async () => {
  const apiFetch = vi.fn<ApiFetch>(async () => textResponse(TLE));
  await expect(satellitesSource(apiFetch).readGroup("stations")).resolves.toEqual({
    ok: true,
    status: 200,
    text: TLE,
  });
  expect(apiFetch).toHaveBeenCalledTimes(1);
  expect(apiFetch.mock.calls[0][0]).toBe("/api/v1/gev/celestrak/stations");
});

test("forwards the abort signal to the transport", async () => {
  const apiFetch = vi.fn<ApiFetch>(async () => textResponse(TLE));
  const controller = new AbortController();
  await satellitesSource(apiFetch).readGroup("gps-ops", { signal: controller.signal });
  expect(apiFetch.mock.calls[0]).toEqual([
    "/api/v1/gev/celestrak/gps-ops",
    { signal: controller.signal },
  ]);
});

test("non-ok response degrades to { ok:false, status, text:\"\" } without throwing", async () => {
  const apiFetch = vi.fn<ApiFetch>(async () => textResponse("upstream boom", 502));
  await expect(satellitesSource(apiFetch).readGroup("starlink")).resolves.toEqual({
    ok: false,
    status: 502,
    text: "",
  });
});

test("unknown group throws TypeError before any fetch", async () => {
  const apiFetch = vi.fn<ApiFetch>(async () => textResponse(TLE));
  await expect(satellitesSource(apiFetch).readGroup("kessler")).rejects.toThrow(
    TypeError,
  );
  await expect(satellitesSource(apiFetch).readGroup("kessler")).rejects.toThrow(
    "Unknown satellite group",
  );
  expect(apiFetch).not.toHaveBeenCalled();
});

test("index.ts registers the real source, not the stub", async () => {
  const apiFetch = vi.fn<ApiFetch>(async () => textResponse(TLE));
  const sources = createIntelHubLayerSources({ apiFetch });
  await expect(
    (
      sources.satellites as {
        readGroup(group: string): Promise<unknown>;
      }
    ).readGroup("visual"),
  ).resolves.toEqual({ ok: true, status: 200, text: TLE });
  expect(apiFetch).toHaveBeenCalledWith("/api/v1/gev/celestrak/visual", {
    signal: undefined,
  });
  expect((sources.satellites as { label: string }).label).toBe(
    "CelesTrak via IntelHub",
  );
});
