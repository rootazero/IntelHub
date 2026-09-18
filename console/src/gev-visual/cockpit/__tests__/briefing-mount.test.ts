import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { mountCockpitBriefing } from "../briefing-mount";
import type { ApiFetch } from "../../../gev-adapters/http";

// Empty-bullet handling is the core T2 finding: hub gev_summary returns 200 +
// bullets:[] + sources:["cache"] because acled/reliefweb/gdelt don't exist yet.
// The adapter must treat that as `summary.empty === true`, not as an error, and
// must NOT retry-storm (D3).

const weatherBody = {
  source: "noaa",
  fetched_at: "2026-09-18T00:00:00Z",
  temperature_c: 21.5,
  wind_speed_kts: 12.3,
  wind_direction_deg: 250,
  precipitation_mm: 0.4,
  cloud_cover_pct: 60,
  visibility_m: 10000,
  pressure_hpa: 1013,
};

const summaryBody = {
  entity_id: "flight:UAL123",
  generated_at: "2026-09-18T00:00:00Z",
  sources: ["cache"],
  bullets: [
    { text: "first bullet", source_url: "https://example.com/a", age_hours: 3 },
    { text: "second bullet", age_hours: 10 },
  ],
  next_refresh_after: "2026-09-18T00:15:00Z",
};

function router(
  routes: Array<{ match: string; status: number; body: unknown }>,
): ApiFetch {
  return vi.fn(async (path: string) => {
    const route = routes.find((r) => path.includes(r.match));
    if (!route) return new Response("not found", { status: 404 });
    return new Response(JSON.stringify(route.body), { status: route.status });
  });
}

const okRoutes = [
  { match: "/gev/weather", status: 200, body: weatherBody },
  { match: "/gev/summary", status: 200, body: summaryBody },
];

describe("mountCockpitBriefing", () => {
  test("constructor contract: rejects a non-function apiFetch", () => {
    expect(() => mountCockpitBriefing(undefined as any)).toThrow(TypeError);
    expect(() => mountCockpitBriefing("x" as any)).toThrow(TypeError);
  });

  test("fetch normalizes weather metrics + summary bullets", async () => {
    const h = mountCockpitBriefing(router(okRoutes));
    const b = await h.fetch(39.9, 116.4, "flight:UAL123");
    expect(b.weather.source).toBe("noaa");
    expect(b.weather.degraded).toBe(false);
    expect(b.weather.metrics).toHaveLength(4);
    expect(b.weather.metrics.map((m) => m.key)).toEqual([
      "temperature",
      "wind",
      "precipitation",
      "visibility",
    ]);
    expect(b.weather.metrics[0].value).toBe("22°C");
    expect(b.weather.metrics[1].value).toBe("12 KTS");
    // wind direction detail via vendor formatCockpitWindDirection (250° → W).
    expect(b.weather.metrics[1].detail).toBe("W · 250°");
    expect(b.weather.metrics[3].value).toBe("10 KM");

    expect(b.summary.entityId).toBe("flight:UAL123");
    expect(b.summary.empty).toBe(false);
    expect(b.summary.bullets).toHaveLength(2);
    expect(b.summary.bullets[0]).toEqual({
      text: "first bullet",
      sourceUrl: "https://example.com/a",
      ageHours: 3,
    });
    expect(b.summary.bullets[1].sourceUrl).toBeNull();
  });

  test("empty bullets resolve as summary.empty=true (hub stub), never throw", async () => {
    const h = mountCockpitBriefing(
      router([
        { match: "/gev/weather", status: 200, body: weatherBody },
        {
          match: "/gev/summary",
          status: 200,
          body: { ...summaryBody, bullets: [], sources: ["cache"] },
        },
      ]),
    );
    const b = await h.fetch(39.9, 116.4, "flight:UAL123");
    expect(b.summary.empty).toBe(true);
    expect(b.summary.bullets).toEqual([]);
    expect(b.summary.sources).toEqual(["cache"]);
    expect(b.summary.degraded).toBe(false);
  });

  test("weather 503 degrades the briefing without rejecting (D3: no retry storm)", async () => {
    const h = mountCockpitBriefing(
      router([
        { match: "/gev/weather", status: 503, body: { error: "down" } },
        { match: "/gev/summary", status: 200, body: summaryBody },
      ]),
    );
    const b = await h.fetch(39.9, 116.4, "flight:UAL123");
    expect(b.weather.degraded).toBe(true);
    expect(b.weather.metrics).toEqual([]);
    expect(b.summary.empty).toBe(false);
  });

  test("summary non-ok marks summary.degraded and still resolves", async () => {
    const h = mountCockpitBriefing(
      router([
        { match: "/gev/weather", status: 200, body: weatherBody },
        { match: "/gev/summary", status: 500, body: { error: "boom" } },
      ]),
    );
    const b = await h.fetch(39.9, 116.4, "flight:UAL123");
    expect(b.summary.degraded).toBe(true);
    expect(b.summary.empty).toBe(true);
  });

  test("pages() exposes the vendor's THREE brief pages (not 6)", () => {
    const h = mountCockpitBriefing(router(okRoutes));
    const pages = h.pages();
    expect(pages).toHaveLength(3);
    expect(pages.map((p) => p.id)).toEqual(["signals", "news", "local"]);
  });

  test("next()/prev() wrap around the bullet list", async () => {
    const h = mountCockpitBriefing(router(okRoutes));
    await h.fetch(39.9, 116.4, "flight:UAL123");
    expect(h.total()).toBe(2);
    expect(h.next()).toBe(1);
    expect(h.next()).toBe(0); // wrap
    expect(h.prev()).toBe(1); // wrap back
  });

  test("rotation auto-advances every COCKPIT_BRIEF_ROTATE_MS and stop() halts it", async () => {
    vi.useFakeTimers();
    try {
      const h = mountCockpitBriefing(router(okRoutes));
      await h.fetch(39.9, 116.4, "flight:UAL123");
      h.start();
      vi.advanceTimersByTime(9000); // COCKPIT_BRIEF_ROTATE_MS = 9000
      expect(h.index()).toBe(1);
      h.stop();
      vi.advanceTimersByTime(9000);
      expect(h.index()).toBe(1); // no further advance
    } finally {
      vi.useRealTimers();
    }
  });

  test("start() with empty bullets is a no-op (graceful empty)", async () => {
    vi.useFakeTimers();
    try {
      const h = mountCockpitBriefing(
        router([
          { match: "/gev/weather", status: 200, body: weatherBody },
          { match: "/gev/summary", status: 200, body: { ...summaryBody, bullets: [] } },
        ]),
      );
      await h.fetch(39.9, 116.4, "flight:UAL123");
      h.start();
      vi.advanceTimersByTime(9000);
      expect(h.index()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  test("destroy stops the timer, clears state, and is idempotent", async () => {
    const h = mountCockpitBriefing(router(okRoutes));
    await h.fetch(39.9, 116.4, "flight:UAL123");
    h.destroy();
    expect(h.current()).toBeNull();
    expect(h.total()).toBe(0);
    expect(() => h.destroy()).not.toThrow();
  });
});
