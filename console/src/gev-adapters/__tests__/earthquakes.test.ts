// Explicit vitest imports (repo convention, cf. ./stubs.test.ts): vitest.config.ts
// sets globals:true for the runtime, but `tsc -b` does not load vitest/globals
// types, so bare `test`/`expect` break `npm run build`.
import { expect, test, vi } from "vitest";
import type { ApiFetch } from "../http";
import { createIntelHubLayerSources } from "../index";
import { earthquakesSource } from "../earthquakes";

// Field-for-field what hub-core api.rs `quake_row` emits for a USGS row
// (`depthKm` is always null: USGS keeps depth in geometry, not properties).
// Sampled from a live `geo_events` quake row.
const ROWS = [
  {
    stableId: "us7000thse",
    usgsId: "us7000thse",
    lon: -76.7822,
    lat: 4.495,
    depthKm: null,
    mag: 4.6,
    place: "23 km SW of Sipí, Colombia",
    time: 1789539486190,
  },
];

const jsonResponse = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status });

test("passes the endpoint array straight through", async () => {
  const apiFetch = vi.fn<ApiFetch>(async () => jsonResponse(ROWS));
  await expect(earthquakesSource(apiFetch).getSnapshot()).resolves.toEqual(ROWS);
  expect(apiFetch).toHaveBeenCalledTimes(1);
  expect(apiFetch.mock.calls[0][0]).toBe("/api/v1/gev/earthquakes");
});

test("forwards the abort signal to the transport", async () => {
  const apiFetch = vi.fn<ApiFetch>(async () => jsonResponse(ROWS));
  const controller = new AbortController();
  await earthquakesSource(apiFetch).getSnapshot({ signal: controller.signal });
  expect(apiFetch.mock.calls[0]).toEqual([
    "/api/v1/gev/earthquakes",
    { signal: controller.signal },
  ]);
});

test("keeps every engine-destructured key on the row", async () => {
  const apiFetch = vi.fn<ApiFetch>(async () => jsonResponse(ROWS));
  const [row] = await earthquakesSource(apiFetch).getSnapshot();
  // Mirrors the destructuring in gev-engine/src/layers/earthquakes/index.js:86.
  const { stableId, usgsId, lon, lat, depthKm, mag, place, time } = row;
  expect({ stableId, usgsId, lon, lat, depthKm, mag, place, time }).toEqual(ROWS[0]);
});

test("non-array body degrades to []", async () => {
  for (const body of [{ error: "boom" }, null, "nope"]) {
    const apiFetch = vi.fn<ApiFetch>(async () => jsonResponse(body));
    await expect(earthquakesSource(apiFetch).getSnapshot()).resolves.toEqual([]);
  }
});

test("non-ok response throws with the status", async () => {
  const apiFetch = vi.fn<ApiFetch>(async () => jsonResponse({ error: "unauthorized" }, 401));
  await expect(earthquakesSource(apiFetch).getSnapshot()).rejects.toThrow(
    "IntelHub earthquakes HTTP 401",
  );
});

test("index.ts registers the real source, not the stub", async () => {
  const apiFetch = vi.fn<ApiFetch>(async () => jsonResponse(ROWS));
  const sources = createIntelHubLayerSources({ apiFetch });
  await expect(
    (sources.earthquakes as { getSnapshot(): Promise<unknown[]> }).getSnapshot(),
  ).resolves.toEqual(ROWS);
  expect(apiFetch).toHaveBeenCalledWith("/api/v1/gev/earthquakes", {
    signal: undefined,
  });
  expect((sources.earthquakes as { label: string }).label).toBe("USGS via IntelHub");
});
