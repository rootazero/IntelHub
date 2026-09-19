// Contract guard suite — the upgrade tripwire that turns vendor syncs from
// blind diffs into governed upgrades (P2 T12).
//
//   c1  contract pinning: parse the vendored engine's SOURCE_METHODS and
//       structural code anchors, then assert IntelHub's adapters still satisfy
//       them. Anchors are identifier-level CODE (property accesses,
//       declarations) — never line numbers or comment text — so they survive
//       comment drift and reformatting but fire the moment an upstream sync
//       changes a load-bearing contract.
//   c2  engine behavior contracts: the degraded semantics the engine's
//       ingestion relies on (503 envelopes, empty groups, contract-misuse
//       TypeError), driven entirely by a mocked apiFetch — zero network.
//   c3  vendor boundary: host code must not reference the engine's
//       local_data internals outside the exceptions declared in
//       console/gev-engine/UPSTREAM.json (the vite.config.ts externalize
//       plugin is a T8-reviewed intentional exception), and the engine alias
//       channel must stay in sync across vite / vitest / tsconfig (Ruling 6).
//
// Explicit vitest imports (repo convention, cf. gev-adapters/__tests__/stubs.test.ts):
// vitest.config.ts sets globals:true for the runtime, but `tsc -b` does not
// load vitest/globals types, so bare `test`/`expect` break `npm run build`.
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, test, vi } from "vitest";
import type { ApiFetch } from "../../gev-adapters/http";
import { createIntelHubLayerSources } from "../../gev-adapters";

const here = dirname(fileURLToPath(import.meta.url));
const consoleRoot = join(here, "..", "..", "..");
const vendor = (rel: string) => join(consoleRoot, "gev-engine", rel);
const readVendor = (rel: string) => readFileSync(vendor(rel), "utf8");
const readConsole = (rel: string) => readFileSync(join(consoleRoot, rel), "utf8");

/** Parse SOURCE_METHODS out of the vendored constructCatalog.js (code structure). */
function parseSourceMethods(src: string): Array<[string, string[]]> {
  const block =
    src.match(/SOURCE_METHODS\s*=\s*Object\.freeze\(\{([\s\S]*?)\}\)/)?.[1] ?? "";
  return [...block.matchAll(/(\w+):\s*\[([^\]]*)\]/g)].map(
    (m) =>
      [
        m[1],
        m[2].match(/'(\w+)'/g)?.map((s) => s.slice(1, -1)) ?? [],
      ] as [string, string[]],
  );
}

// ── c1: contract pinning ───────────────────────────────────────────────────

describe("c1: contract pinning (upstream churn fuse)", () => {
  test("adapters cover upstream SOURCE_METHODS verbatim, in both directions", () => {
    const entries = parseSourceMethods(
      readVendor("src/app/constructCatalog.js"),
    );
    expect(entries.length).toBeGreaterThanOrEqual(14);
    const sources = createIntelHubLayerSources({
      apiFetch: async () => new Response("{}"),
    });
    for (const [layer, methods] of entries)
      for (const m of methods)
        expect(
          typeof (sources as Record<string, any>)[layer]?.[m],
          `${layer}.${m}`,
        ).toBe("function");

    // Reverse direction: no factory entry outside the pinned layer set.
    // `transit` is the sole documented extra — it is absent from
    // SOURCE_METHODS (the engine layer validates it itself,
    // layers/transit/index.js:34-41) but the catalog consumes
    // sources.transit, so the factory must register it (T3 I-2).
    const upstreamLayers = new Set(entries.map(([layer]) => layer));
    for (const key of Object.keys(sources)) {
      if (key === "transit") continue;
      expect(upstreamLayers.has(key), `factory entry outside SOURCE_METHODS: ${key}`).toBe(
        true,
      );
    }
  });

  test("flights/military ingestion still reads the pinned snapshot keys", () => {
    // Keys the engine's ingestion destructures off our envelopes
    // (flights/ingestion.js:35-53, military/ingestion.js:34-52, verified T6).
    // Anchored on `snapshot.<key>` property accesses in vendor code. The two
    // layers deliberately differ — military keys freshness off observedAtMs +
    // stale and never reads ageMs — so the pin is per-file, not a shared set.
    const pins: Array<[string, string[]]> = [
      [
        "src/layers/flights/ingestion.js",
        [
          "status",
          "observedAtMs",
          "ageMs",
          "stale",
          "freshness",
          "source",
          "coverage",
        ],
      ],
      [
        "src/layers/military/ingestion.js",
        ["status", "observedAtMs", "stale", "freshness", "source", "reason"],
      ],
    ];
    for (const [rel, keys] of pins) {
      const src = readVendor(rel);
      for (const k of keys)
        expect(src, `${rel} reads snapshot.${k}`).toMatch(
          new RegExp(`snapshot\\.${k}\\b`),
        );
    }
  });

  test("catalog still validates sources and throws Invalid catalog source", () => {
    const src = readVendor("src/app/constructCatalog.js");
    // The exact typeof guard the catalog runs per SOURCE_METHODS entry.
    expect(src).toMatch(
      /typeof sources\?\.\[name\]\?\.\[method\] !== 'function'/,
    );
    expect(src).toMatch(/Invalid catalog source/);
  });

  test("satellites ingestion still resolves readGroup and maps non-ok to an empty group", () => {
    const src = readVendor("src/layers/satellites/ingestion.js");
    expect(src).toMatch(/source\.readGroup\(/);
    // Non-ok must degrade to `{ entries: [], ok: false }` — the adapter's
    // contract is "transport failures resolve, never throw".
    expect(src).toMatch(/!res\.ok\)\s*return\s*\{[^}]*entries:\s*\[\][^}]*ok:\s*false/);
  });

  test("earthquakes update still destructures the pinned row fields", () => {
    const src = readVendor("src/layers/earthquakes/index.js");
    const destructure = src.match(/for \(const \{([\s\S]*?)\} of rows\)/)?.[1] ?? "";
    expect(destructure).not.toBe("");
    for (const f of [
      "stableId",
      "usgsId",
      "lon",
      "lat",
      "depthKm",
      "mag",
      "place",
      "time",
    ])
      expect(destructure, `row field ${f}`).toMatch(new RegExp(`\\b${f}\\b`));
  });

  test("vessels ingestion still reads the pinned envelope keys", () => {
    const src = readVendor("src/layers/vessels/ingestion.js");
    for (const k of ["records", "observedAtMs", "freshness", "complete"])
      expect(src, `vessels reads snapshot.${k}`).toMatch(
        new RegExp(`snapshot\\.${k}\\b`),
      );
  });

  // ── GEV P3 anchors (T15): traffic / cctv / installations vendor seams ──

  test("traffic source still posts QL to /api/overpass and reads tomtom status/flow", () => {
    const src = readVendor("src/layers/traffic/source.js");
    expect(src).toContain("/api/overpass");
    expect(src).toContain("[out:json]");
    expect(src).toContain("/api/tomtom/status");
    expect(src).toMatch(/hasKey/);
    const flow = readVendor("src/layers/traffic/flowSource.js");
    expect(flow).toContain("/api/tomtom/flow/");
    expect(flow).toMatch(/\.pbf/);
    const decode = readVendor("src/layers/traffic/flowDecode.js");
    expect(decode).toContain("Traffic flow"); // MVT layer name
    expect(decode).toMatch(/traffic_level/);
  });

  test("cctv source still hits the four /api/cctv endpoints and mp4/hls/webm video set", () => {
    const policy = readVendor("src/layers/cctv/sourcePolicy.js");
    for (const k of ["/api/cctv/frame", "/api/cctv/sources", "/api/cctv/health", "/api/cctv/media"])
      expect(policy, `sourcePolicy pins ${k}`).toContain(k);
    const model = readVendor("src/layers/cctv/model.js");
    // T8 review M1 anchor: mjpeg must NOT be a video feed type
    expect(model).toMatch(/isVideoFeedType/);
    expect(model).not.toMatch(/mjpeg['"]\s*\)/); // no mjpeg in the video branch call
  });

  test("installations source still reads /api/military-installations with elements+saturated envelope", () => {
    const src = readVendor("src/layers/installations/source.js");
    expect(src).toContain("/api/military-installations");
    expect(src).toMatch(/payload\?\.elements/);
    expect(src).toMatch(/saturated/);
    expect(src).toMatch(/elementCap/);
  });

  test("wave-1 snapshot status is a number across fresh/stale/degraded — never pinned to 200 (T6 M-3)", async () => {
    // Ruling T6 M-3: a stale upstream snapshot is INTENTIONALLY framed as
    // status 503 (aircraft-map.ts toEnvelope — "stale-200 responses get 503"),
    // so the contract under test is "status is an HTTP-ish number", never
    // "status === 200". This test deliberately exercises a fresh body and the
    // hub's degraded {stale:true} body and only asserts the numeric contract.
    const fresh = {
      ts: new Date(Date.now() - 30_000).toISOString(),
      aircraft: [],
    };
    for (const body of [fresh, { stale: true, aircraft: [] }]) {
      const apiFetch: ApiFetch = async () =>
        new Response(JSON.stringify(body), { status: 200 });
      const s = createIntelHubLayerSources({ apiFetch });
      const env = await (s as Record<string, any>).flights.getSnapshot();
      expect(typeof env.status, JSON.stringify(Object.keys(body))).toBe("number");
      expect(Number.isInteger(env.status)).toBe(true);
    }
    // The degraded hub body must produce 503 (the intentional deviation a
    // 200-only assertion would forbid) while staying a number.
    const degraded = createIntelHubLayerSources({
      apiFetch: async () =>
        new Response(JSON.stringify({ stale: true, aircraft: [] }), {
          status: 200,
        }),
    });
    const staleEnv = await (degraded as Record<string, any>).flights.getSnapshot();
    expect(staleEnv.status).toBe(503);
    expect(typeof staleEnv.status).toBe("number");
  });

  // ── GEV P6 anchors (T1): visual-presets render-core import surface ──

  test("visual presets render-core exports are pinned", () => {
    const presets = readVendor("src/ui/visualPresets.js");
    expect(presets).toMatch(/export const TRANSITION_DURATION_MS = 500/);
    expect(presets).toMatch(/export const STYLES\b/);
    expect(presets).toMatch(/export const STYLE_PRESET_DEFAULTS\b/);
    expect(presets).toMatch(/export const STYLE_STATUS_LABELS\b/);
    expect(presets).toMatch(/export const SHARPEN_SHADER\b/);
    for (const [key, shader] of [
      ["retro", "retroShader"],
      ["surveillance", "nightVisionShader"],
      ["thermal", "thermalShader"],
      ["anime", "animeShader"],
      ["noir", "noirShader"],
      ["snow", "snowShader"],
    ] as const) {
      expect(presets, `STYLES.${key}`).toMatch(new RegExp(`${key}\\s*:\\s*${shader}\\b`));
    }
    const fx = readVendor("src/ui/visualEffects.js");
    expect(fx).toMatch(/export class VisualEffects\b/);
    for (const m of [
      "initStyles",
      "initPostProcess",
      "setStageIntensity",
      "startTransition",
      "applyBloomIntensity",
      "setBloomEnabled",
      "applySharpenIntensity",
      "setSharpenEnabled",
      "stop",
      "destroy",
    ]) {
      expect(fx, `VisualEffects.${m}`).toMatch(new RegExp(`\\n  ${m}\\(`));
    }
    const bloom = readVendor("src/bloom.js");
    for (const e of ["BLOOM_INTENSITY_DEFAULT", "clampBloomIntensity", "bloomStrengthFromIntensity"]) {
      expect(bloom).toMatch(new RegExp(`export (const|function) ${e}\\b`));
    }
    for (const s of ["retro", "surveillance", "thermal", "anime", "noir", "snow"]) {
      expect(readVendor(`src/styles/${s}.js`)).toMatch(/export const \w+Shader\b/);
    }
  });

  // ── GEV P7 anchors (T1): camera orientation + location search import surface ──

  test("P7 camera orientation + location search render-core exports are pinned", () => {
    const cam = readVendor("src/ui/cameraOrientationControls.js");
    for (const e of [
      "OBLIQUE_PITCH", "STRAIGHT_DOWN_PITCH", "pickViewTarget",
      "readCameraTargetFrame", "setCameraTargetFrame", "frameIsTilted",
      "toggleCameraTilt", "resetCameraNorth", "createCameraOrientationAnimator",
    ]) {
      expect(cam, `cameraOrientationControls.${e}`).toMatch(
        new RegExp(`export (const|function) ${e}\\b`),
      );
    }
    // camera core must never gain a scopeMask/celestialRing dependency (圆圈遮罩禁令)
    expect(cam).not.toMatch(/scopeMask|celestialRing/);

    const ls = readVendor("src/ui/locationSearch.js");
    expect(ls).toMatch(/export class LocationSearch\b/);

    const loc = readVendor("src/locations.js");
    expect(loc).toMatch(/export async function searchAndFlyTo\b/);
    // shell-singleton import we must ALWAYS override via options.features /
    // options.recoverNearView — pin it so upstream changes here trip the guard
    expect(loc).toMatch(/import\s*\{[^}]*applicationServices[^}]*\}\s*from/);

    // layer tracking APIs the follow button drives
    const flights = readVendor("src/layers/flights/queries.js");
    for (const m of ["trackById", "stopTracking", "getTrackedInfo"]) {
      expect(flights, `flights.${m}`).toMatch(new RegExp(`${m}\\s*\\(`));
    }
    const sats = readVendor("src/layers/satellites/controls.js");
    for (const m of ["trackById", "stopTracking", "getTrackedInfo"]) {
      expect(sats, `satellites.${m}`).toMatch(new RegExp(`${m}\\s*\\(`));
    }
  });

  // ── GEV P8 anchors (T1): annotation draw surface import contract ──

  test("P8 drawMode pure-function exports are pinned", async () => {
    // drawMode.js is the vendor's explicitly-pure half ("No Cesium, no DOM —
    // importable under `node --test`"), so import it for real and pin the
    // runtime VALUES the draw adapters branch on, not merely their names.
    const m = (await import("gev-engine/src/annotations/drawMode.js")) as Record<
      string,
      unknown
    >;
    for (const name of [
      "createDrawSession",
      "addVertex",
      "finishSpec",
      "normalizeShape",
      "ringAreaM2",
      "greatCircleM",
      "MIN_VERTICES",
      "DRAW_SHAPES",
    ]) {
      expect(m, `drawMode.${name}`).toHaveProperty(name);
    }
    expect(m.DRAW_SHAPES).toEqual(["area", "line", "pin"]);
    expect(m.MIN_VERTICES).toEqual({ area: 3, line: 2, pin: 1 });
  });

  test("P8 annotationEngine factory + helpers are pinned", () => {
    // Source anchors, not a live import: annotationEngine.js transitively pulls
    // data/neighborhoodPolygons.js, which imports an un-vendored
    // `local_data/*.json` that only vite.config.ts externalizes (vitest.config
    // deliberately does not carry that plugin). Same seam style as P6/P7.
    const src = readVendor("src/annotations/annotationEngine.js");
    for (const e of [
      "createAnnotationEngine",
      "normalizeTargetKey",
      "resolveOutlineWithRetry",
    ]) {
      expect(src, `annotationEngine.${e}`).toMatch(
        new RegExp(`export (async function|function|const) ${e}\\b`),
      );
    }
    // The manual-draw integration seam: the engine consumes drawMode's ring
    // helpers, so a hand-drawn shape flows through the same spec pipeline.
    expect(src).toMatch(/from '\.\/drawMode\.js'/);
    expect(src).toMatch(/ringCentroid/);
    // 圆圈遮罩禁令 (P7): the annotation engine must not adopt a scopeMask.
    expect(src).not.toMatch(/scopeMask/);
  });

  test("P8 renderers export their constructors", async () => {
    const factories: Array<[string, string]> = [
      ["gev-engine/src/annotations/hybridAnnotationRenderer.js", "createHybridAnnotationRenderer"],
      ["gev-engine/src/annotations/worldAnnotationRenderer.js", "createWorldAnnotationRenderer"],
      ["gev-engine/src/annotations/screenAnnotationRenderer.js", "createScreenAnnotationRenderer"],
    ];
    for (const [path, factory] of factories) {
      const m = (await import(/* @vite-ignore */ path)) as Record<string, unknown>;
      expect(typeof m[factory], `${path} → ${factory}`).toBe("function");
    }
  });

  // ── GEV P9 anchors (T1): cockpit render-core import surface ──
  //
  // The plan's guessed export names (cockpitMath heading/altitudeMeters/
  // speedMps/cockpitCloudCover/cockpitCloudLightning, cockpitPresentation at
  // src/ root, TARGET_STYLE_BY_MODE exported, 6 brief pages) are ALL wrong.
  // These pins are written against the vendored files themselves:
  //   src/cockpitMath.js, src/cockpitVisionPolicy.js,
  //   src/ui/cockpitPresentation.js, src/ui/cockpitCamera.js.

  test("P9 cockpitMath compass/ruler/geometry exports are pinned (pure module)", async () => {
    // cockpitMath + panelRailGeometry + cockpitVisionPolicy + (ui/)cockpitPresentation
    // have ZERO imports, so they are live-imported (runtime VALUES, not just
    // names) — the drawMode.js style (P8).
    const m = (await import("gev-engine/src/cockpitMath.js")) as Record<string, any>;
    const fns = [
      "normalizeHeading",
      "slewHeading",
      "cockpitAnchorCorrectionStep",
      "cockpitUiUpdateDue",
      "cockpitSurfaceWaitExpired",
      "cockpitGroundSafeHeight",
      "cockpitAltitudeDisplayFt",
      "formatCockpitContextScope",
      "compassDivisions",
      "formatCompassDivision",
      "altitudeRulerStep",
      "altitudeRulerTicks",
      "altitudeRulerCurveInset",
      "formatAltitudeRulerTick",
      "speedRulerStep",
      "speedRulerTicks",
      "formatSpeedRulerTick",
      "bearingBetweenCoordinates",
      "relativeBearing",
      "resolveTrackedAircraftInfo",
      "resolveCockpitContextReadout",
      "resolveHudRailLayout", // re-exported from ./ui/panelRailGeometry.js
    ];
    for (const name of fns)
      expect(typeof m[name], `cockpitMath.${name}`).toBe("function");
    // Arity pins: the HUD adapters call these positionally.
    for (const [name, arity] of [
      ["normalizeHeading", 1],
      ["slewHeading", 3],
      ["cockpitAnchorCorrectionStep", 3],
      ["cockpitUiUpdateDue", 3],
      ["cockpitSurfaceWaitExpired", 2],
      ["cockpitGroundSafeHeight", 3],
      ["cockpitAltitudeDisplayFt", 2],
      ["formatCockpitContextScope", 2],
      ["compassDivisions", 1],
      ["formatCompassDivision", 1],
      ["altitudeRulerStep", 1],
      ["altitudeRulerTicks", 1],
      ["altitudeRulerCurveInset", 1],
      ["formatAltitudeRulerTick", 1],
      ["speedRulerStep", 1],
      ["speedRulerTicks", 1],
      ["formatSpeedRulerTick", 1],
      ["bearingBetweenCoordinates", 4],
      ["relativeBearing", 2],
      ["resolveTrackedAircraftInfo", 0],
      ["resolveCockpitContextReadout", 0],
    ] as const)
      expect(m[name].length, `cockpitMath.${name} arity`).toBe(arity);
    // Behavior pins — the instrument readouts the HUD renders verbatim.
    expect(m.normalizeHeading(370)).toBe(10);
    expect(m.compassDivisions(5)).toEqual([270, 300, 330, 0, 30, 60, 90]);
    expect(m.formatCompassDivision(0)).toBe("N");
    expect(m.formatCompassDivision(45)).toBe("NE");
    expect(m.altitudeRulerTicks(1234)).toHaveLength(9);
    expect(m.formatAltitudeRulerTick(1234)).toBe("01234");
    expect(m.speedRulerTicks(120)).toHaveLength(9);
    expect(m.formatSpeedRulerTick(7)).toBe("007");
    expect(m.cockpitAltitudeDisplayFt(1000, true)).toBe(0);
  });

  test("P9 cockpitVisionPolicy 5-mode gating exports + arities are pinned", async () => {
    const m = (await import("gev-engine/src/cockpitVisionPolicy.js")) as Record<string, any>;
    expect(m.COCKPIT_VISION_MODES).toEqual([
      "optical",
      "crt",
      "nvg",
      "thermal",
      "noir",
    ]);
    for (const [name, arity] of [
      ["normalizeCockpitVisionMode", 1],
      ["captureCockpitVisionBaseline", 2],
      // (stages, mode, restore = {}) → the 3rd arg is optional but load-bearing
      ["applyCockpitVisionStageIntensities", 2],
    ] as const) {
      expect(typeof m[name], `cockpitVisionPolicy.${name}`).toBe("function");
      expect(m[name].length, `cockpitVisionPolicy.${name} arity`).toBe(arity);
    }
    expect(m.normalizeCockpitVisionMode("nvg")).toBe("nvg");
    expect(m.normalizeCockpitVisionMode("bogus")).toBe("optical");
    // optical is the restore branch: it writes back the captured baselines
    // and returns null (no temporary style to surface).
    const stages = () => ({
      retro: { uniforms: { intensity: 0.25 } },
      surveillance: { uniforms: { intensity: 0.5 } },
      thermal: { uniforms: { intensity: 0.75 } },
    });
    const s1 = stages();
    expect(m.applyCockpitVisionStageIntensities(s1, "optical", { retro: 0.2 })).toBeNull();
    expect(s1.retro.uniforms.intensity).toBe(0.2);
    // A real mode zeroes EVERY stage then lights exactly one target style
    // (thermal → 'thermal'); a mode whose target style is absent returns null.
    const s2 = stages();
    expect(m.applyCockpitVisionStageIntensities(s2, "thermal")).toBe("thermal");
    expect(s2.thermal.uniforms.intensity).toBe(1);
    expect(s2.retro.uniforms.intensity).toBe(0);
    expect(s2.surveillance.uniforms.intensity).toBe(0);
    expect(m.applyCockpitVisionStageIntensities(stages(), "crt")).toBe("retro");
    // The plan's T3 calls `TARGET_STYLE_BY_MODE[mode]` — it is module-PRIVATE,
    // so the adapter must go through applyCockpitVisionStageIntensities instead.
    expect(m).not.toHaveProperty("TARGET_STYLE_BY_MODE");
  });

  test("P9 cockpitPresentation cadences + 3-page brief table are pinned", async () => {
    const m = (await import("gev-engine/src/ui/cockpitPresentation.js")) as Record<string, any>;
    // Cadences the HUD mirrors (P9 T4): camera 20Hz, HUD 10Hz, context 4Hz.
    expect(m.COCKPIT_CAMERA_UPDATE_MS).toBe(50);
    expect(m.COCKPIT_HUD_UPDATE_MS).toBe(100);
    expect(m.COCKPIT_CONTEXT_UPDATE_MS).toBe(250);
    expect(m.COCKPIT_BRIEF_ROTATE_MS).toBe(9000); // plan guessed 6000
    expect(m.COCKPIT_REGIONAL_REFRESH_MS).toBe(300_000);
    expect(m.COCKPIT_REGIONAL_REFRESH_DISTANCE_M).toBe(25_000);
    for (const name of [
      "COCKPIT_HEADING_SLEW_DPS",
      "COCKPIT_FORWARD_OFFSET_M",
      "COCKPIT_UP_OFFSET_M",
      "COCKPIT_MIN_GROUND_CLEARANCE_M",
      "COCKPIT_VIEW_PITCH_DEG",
      "COCKPIT_GROUND_PROBE_MS",
      "COCKPIT_GROUND_WAIT_TIMEOUT_MS",
      "COCKPIT_BRIEF_CYCLE_OFF_HELP",
      "COCKPIT_BRIEF_CYCLE_ON_HELP",
    ])
      expect(m, `cockpitPresentation.${name}`).toHaveProperty(name);
    // Vendor ships THREE pages (signals/news/local), not the plan's 6.
    expect(m.COCKPIT_BRIEF_PAGES).toHaveLength(3);
    expect(m.COCKPIT_BRIEF_PAGES.map((p: any) => p.id)).toEqual([
      "signals",
      "news",
      "local",
    ]);
    for (const [name, arity] of [
      ["isRenderedOnScreen", 1],
      ["formatCockpitBriefAge", 1],
      ["formatCockpitWindDirection", 1],
      // (element, text, numericValue, {circularRange,immediate} = {})
      ["setCockpitRollingValue", 3],
    ] as const) {
      expect(typeof m[name], `cockpitPresentation.${name}`).toBe("function");
      expect(m[name].length, `cockpitPresentation.${name} arity`).toBe(arity);
    }
  });

  test("P9 cockpitCamera stays a Cesium-bound zero-arity mixin method", () => {
    // Source anchors, not a live import: cockpitCamera.js imports 'cesium'
    // directly. The plan's `update(viewer, entity, params)` guess is wrong —
    // the module exports ONE zero-arity method meant to be `.call()`ed with a
    // cockpit controller as `this` (it reads this.viewer/this.trackedEntity).
    const src = readVendor("src/ui/cockpitCamera.js");
    expect(src).toMatch(/export function update\(\)/);
    expect(src).toMatch(/^import \* as Cesium from 'cesium';/m);
    expect(src).not.toMatch(/export class/);
    // Coupling pin: it consumes the pinned cockpitMath helpers + presentation
    // cadences, so an upstream refactor that relocates either trips here.
    for (const helper of [
      "slewHeading",
      "cockpitAnchorCorrectionStep",
      "cockpitGroundSafeHeight",
      "cockpitSurfaceWaitExpired",
      "cockpitUiUpdateDue",
    ])
      expect(src, `cockpitCamera uses ${helper}`).toMatch(
        new RegExp(`\\b${helper}\\b`),
      );
    expect(src).toMatch(/COCKPIT_CAMERA_UPDATE_MS|COCKPIT_HUD_UPDATE_MS/);
  });

  test("P9 mountVisualEffects handle exposes getStages() as a Map (P6 handle extension)", async () => {
    const { mountVisualEffects } = await import("../../gev-visual/visual-effects");
    const added: any[] = [];
    const viewer = {
      scene: {
        requestRender: () => {},
        postProcessStages: {
          bloom: { enabled: true, uniforms: {} },
          add(stage: any) {
            added.push(stage);
          },
          remove(stage: any) {
            const i = added.indexOf(stage);
            if (i >= 0) added.splice(i, 1);
          },
        },
      },
    };
    const handle = mountVisualEffects(viewer as any, {
      createStage: (options: any) => ({
        ...options,
        enabled: true,
        uniforms: { ...(options.uniforms ?? {}) },
      }),
      requestFrame: () => 0,
      cancelFrame: () => {},
      now: () => 0,
    });
    const stages = handle.getStages();
    expect(stages).toBeInstanceOf(Map);
    // Keyed by vendor STYLE name (retro/surveillance/thermal/anime/noir/snow),
    // NOT by the Cesium stage name `godsEyeView_<style>`.
    expect([...(stages?.keys() ?? [])].sort()).toEqual([
      "anime",
      "noir",
      "retro",
      "snow",
      "surveillance",
      "thermal",
    ]);
    expect(stages?.get("retro")?.uniforms.intensity).toBe(0);
    // The vendor policy consumes a plain object — the documented bridge must
    // keep working across the Map conversion.
    expect(Object.fromEntries(stages ?? [])).toHaveProperty("retro");
    handle.destroy();
    expect(handle.getStages()).toBeNull();
  });

  // ── GEV P10 anchors (T1): HUD-tail render-core import surface ──
  //
  // The plan's guessed export names for this batch are NOT the source of
  // truth (e.g. `godsEyeView.v6.layoutRightPanels` never existed — the second
  // panel storage family is `panelCollapsed`). Every pin below is written
  // against the vendored file itself, and every module the plan names gets at
  // least one assertion (14 modules over 13 tests + the storage guard).

  test("P10 panelDisclosure bind/escape/hover exports + guard TypeErrors are pinned", async () => {
    const m = (await import("gev-engine/src/ui/panelDisclosure.js")) as Record<string, any>;
    for (const [name, arity] of [
      ["bindPanelDisclosure", 1],
      ["collapsePanelOnEscape", 2],
      ["createHoverDisclosure", 1],
    ] as const) {
      expect(typeof m[name], `panelDisclosure.${name}`).toBe("function");
      expect(m[name].length, `panelDisclosure.${name} arity`).toBe(arity);
    }
    // The vendor's own guard messages are what the adapter surfaces — pin them
    // so an upstream softening of the guards is visible here.
    expect(() => m.bindPanelDisclosure({})).toThrow(
      /Panel disclosure requires a panel, onChange and onEscape/,
    );
    expect(() => m.createHoverDisclosure({})).toThrow(
      /Hover disclosure requires a panel, document, onChange and onEscape/,
    );
    // bindPanelDisclosure returns a destroy-only handle (idempotent cleanup).
    const panel = document.createElement("div");
    const handle = m.bindPanelDisclosure({
      panel,
      onChange: () => {},
      onEscape: () => {},
    });
    expect(typeof handle.destroy).toBe("function");
    handle.destroy();
    expect(() => handle.destroy()).not.toThrow();
  });

  test("P10 panelMeasurement + panelRails barrel surface is pinned", async () => {
    const measurement = (await import(
      "gev-engine/src/ui/panelMeasurement.js"
    )) as Record<string, any>;
    expect(typeof measurement.measurePanelNaturalHeight).toBe("function");
    expect(measurement.measurePanelNaturalHeight.length).toBe(2);
    // No non-glow child → unconstrained scrollHeight is the natural height.
    expect(
      measurement.measurePanelNaturalHeight(
        {
          children: [],
          scrollHeight: 42.3,
          getBoundingClientRect: () => ({ height: 0 }),
        },
        () => ({}),
      ),
    ).toBe(43);

    const rails = (await import("gev-engine/src/ui/panelRails.js")) as Record<string, any>;
    for (const name of [
      "layoutLeftPanelRail",
      "layoutRightPanelRail",
      "measurePanelNaturalHeight",
      "resolveHudRailLayout",
      "shouldHideCollapsedRightPanels",
      "allocatePanelStackHeights",
      "panelStackAutoCollapseIndices",
      "resolveLeftStackBottomBoundary",
      "resolvePanelStackCorridor",
    ])
      expect(typeof rails[name], `panelRails.${name}`).toBe("function");
    // Barrel IDENTITY: re-exports are the same function objects, so an upstream
    // rename trips here before the P10 adapters ever import the module.
    expect(rails.measurePanelNaturalHeight).toBe(
      measurement.measurePanelNaturalHeight,
    );
    const geometry = (await import(
      "gev-engine/src/ui/panelRailGeometry.js"
    )) as Record<string, any>;
    expect(rails.resolveHudRailLayout).toBe(geometry.resolveHudRailLayout);
    expect(rails.shouldHideCollapsedRightPanels).toBe(
      geometry.shouldHideCollapsedRightPanels,
    );
  });

  test("P10 panelRailGeometry rail slot + collapse-visibility decisions are pinned", async () => {
    const m = (await import(
      "gev-engine/src/ui/panelRailGeometry.js"
    )) as Record<string, any>;
    expect(m.resolveHudRailLayout.length).toBe(1);
    expect(m.shouldHideCollapsedRightPanels.length).toBe(1);
    // Malformed input resolves null rather than throwing — the adapter relies
    // on the null branch to fall back to a static layout.
    expect(m.resolveHudRailLayout({})).toBeNull();
    const centered = m.resolveHudRailLayout({
      viewportHeight: 1000,
      panelHeight: 200,
      laneLeft: 0,
      laneRight: 100,
      baseTop: 0,
      baseBottom: 1000,
    });
    expect(centered).toMatchObject({ top: 400, maxHeight: 1000, constrained: false });
    // An obstacle above the viewport midpoint pushes the safe top down.
    const pushed = m.resolveHudRailLayout({
      viewportHeight: 1000,
      panelHeight: 200,
      laneLeft: 0,
      laneRight: 100,
      baseTop: 0,
      baseBottom: 1000,
      obstacles: [{ left: 0, right: 100, top: 0, bottom: 300 }],
    });
    expect(pushed.safeTop).toBe(312);
    expect(
      m.shouldHideCollapsedRightPanels({
        hudVariant: "tactical",
        hasExpandedPanel: true,
      }),
    ).toBe(true);
    expect(
      m.shouldHideCollapsedRightPanels({
        hudVariant: "tactical",
        hasExpandedPanel: false,
      }),
    ).toBe(false);
    expect(
      m.shouldHideCollapsedRightPanels({
        hudVariant: "minimal",
        hasExpandedPanel: true,
      }),
    ).toBe(false);
  });

  test("P10 SceneControls class contract + method list are pinned", async () => {
    const m = (await import(
      "gev-engine/src/ui/sceneControls.js"
    )) as Record<string, any>;
    expect(typeof m.SceneControls).toBe("function");
    expect(m.SceneControls.length).toBe(1); // single destructured options bag
    for (const name of [
      "present",
      "listen",
      "run",
      "renderSceneSelect",
      "renderShotList",
      "setButtons",
      "setProgress",
      "updateStatus",
      "updateRuntime",
      "setPlaybackActive",
      "setPlaybackKeyboardEnabled",
      "destroy",
    ])
      expect(
        typeof m.SceneControls.prototype[name],
        `SceneControls.${name}`,
      ).toBe("function");
    // Empty elements bag = "no scene panel on this page": the ctor must bail
    // out without binding anything, and destroy must stay idempotent.
    const controls = new m.SceneControls({
      read: () => ({ scenes: [], selectedSceneId: null }),
      actions: {},
      elements: {},
    });
    controls.destroy();
    expect(() => controls.destroy()).not.toThrow();
  });

  test("P10 sceneSharing dialog factory + panel mount signatures are pinned", async () => {
    const m = (await import(
      "gev-engine/src/ui/sceneSharing.js"
    )) as Record<string, any>;
    expect(typeof m.createSceneDialog).toBe("function");
    expect(m.createSceneDialog.length).toBe(2); // (title, onClose)
    expect(typeof m.mountSceneSharing).toBe("function");
    // ({ edit, share }, panel = document.getElementById('scene-panel'))
    expect(m.mountSceneSharing.length).toBe(1);
    // A page without a scene panel must degrade to a no-op disposer, never a
    // throw — GlobeV2 calls this unguarded.
    const dispose = m.mountSceneSharing(
      { edit: () => {}, share: () => {} },
      null,
    );
    expect(typeof dispose).toBe("function");
    expect(() => dispose()).not.toThrow();
  });

  test("P10 scenePresentation element map + presenters are pinned", async () => {
    const m = (await import(
      "gev-engine/src/ui/scenePresentation.js"
    )) as Record<string, any>;
    for (const [name, arity] of [
      ["sceneElements", 0], // (root = document)
      ["renderSceneOptions", 2],
      ["renderSceneShots", 3],
      ["presentSceneSelection", 2],
      ["presentSceneButtons", 3],
      ["presentSceneProgress", 2],
      ["presentSceneRuntime", 2],
    ] as const) {
      expect(typeof m[name], `scenePresentation.${name}`).toBe("function");
      expect(m[name].length, `scenePresentation.${name} arity`).toBe(arity);
    }
    const elements = m.sceneElements(document);
    expect(Object.keys(elements).sort()).toEqual([
      "capture",
      "delete",
      "download",
      "export",
      "file",
      "import",
      "new",
      "next",
      "panel",
      "progress",
      "runtime",
      "select",
      "shots",
      "start",
      "status",
      "stop",
      "update",
    ]);
    const fill = document.createElement("div");
    m.presentSceneProgress(fill, 0.5);
    expect(fill.style.width).toBe("50%");
    expect(fill.textContent).toBe("50%");
    const runtime = document.createElement("div");
    m.presentSceneRuntime(runtime, "");
    expect(runtime.classList.contains("active")).toBe(false);
  });

  test("P10 ShareRestoration class signature is pinned (pin-only, D1: no adapter import)", async () => {
    const m = (await import(
      "gev-engine/src/ui/shareRestoration.js"
    )) as Record<string, any>;
    expect(typeof m.ShareRestoration).toBe("function");
    expect(m.ShareRestoration.length).toBe(1);
    for (const name of [
      "attachLinks",
      "start",
      "connect",
      "cancelSelection",
      "destroy",
    ])
      expect(
        typeof m.ShareRestoration.prototype[name],
        `ShareRestoration.${name}`,
      ).toBe("function");
    // D1 ruling: IntelHub does NOT instantiate this vendor class (its React
    // hook owns share restoration), so there is intentionally no adapter test.
    expect(m.ShareRestoration.prototype.start.length).toBe(0);
  });

  test("P10 scenes barrel re-exports the SceneControls owner", async () => {
    const scenes = (await import("gev-engine/src/ui/scenes.js")) as Record<string, any>;
    const controls = (await import(
      "gev-engine/src/ui/sceneControls.js"
    )) as Record<string, any>;
    expect(typeof scenes.SceneControls).toBe("function");
    expect(scenes.SceneControls).toBe(controls.SceneControls);
  });

  test("P10 RecordingControls ctor + lifecycle surface is pinned", async () => {
    const m = (await import(
      "gev-engine/src/ui/recordingControls.js"
    )) as Record<string, any>;
    expect(typeof m.RecordingControls).toBe("function");
    expect(m.RecordingControls.length).toBe(1); // ({ syncShareState })
    expect(typeof m.RecordingControls.prototype.setRecordingMode).toBe("function");
    // (enabled, options = {}) — the defaulted 2nd arg is load-bearing.
    expect(m.RecordingControls.prototype.setRecordingMode.length).toBe(1);
    const controls = new m.RecordingControls({ syncShareState: () => {} });
    expect(controls.destroyed).toBe(false);
    controls.destroy();
    expect(controls.destroyed).toBe(true);
  });

  test("P10 applicationShortcuts binding + key map are pinned", async () => {
    const m = (await import(
      "gev-engine/src/ui/applicationShortcuts.js"
    )) as Record<string, any>;
    expect(typeof m.bindApplicationShortcuts).toBe("function");
    expect(m.bindApplicationShortcuts.length).toBe(1); // single options bag
    const actions = {
      setStyle: vi.fn(),
      dismissSearch: vi.fn(),
      toggleHud: vi.fn(),
      toggleOrbit: vi.fn(),
      toggleCleanView: vi.fn(),
      toggleLayers: vi.fn(),
      cycleDetection: vi.fn(),
      toggleCctv: vi.fn(),
    };
    const listeners: Array<(event: any) => void> = [];
    const docRef = {
      addEventListener: (_type: string, fn: (event: any) => void) => {
        listeners.push(fn);
      },
      removeEventListener: (_type: string, fn: (event: any) => void) => {
        const i = listeners.indexOf(fn);
        if (i >= 0) listeners.splice(i, 1);
      },
    };
    const handle = m.bindApplicationShortcuts({
      documentRef: docRef,
      searchInput: null,
      actions,
    });
    const press = (
      key: string,
      target: any = { matches: () => false },
    ) => listeners.forEach((fn) => fn({ key, target, defaultPrevented: false }));
    // STYLE_KEYS is a private const → pinned through behavior (P9 precedent).
    for (const [key, style] of [
      ["1", "normal"],
      ["2", "retro"],
      ["3", "surveillance"],
      ["4", "thermal"],
      ["5", "anime"],
      ["6", "noir"],
      ["7", "snow"],
    ] as const) {
      press(key);
      expect(actions.setStyle).toHaveBeenLastCalledWith(style);
    }
    press("f");
    expect(actions.toggleLayers).toHaveBeenCalledTimes(1);
    // Form controls keep native typing except for Escape.
    press("f", { matches: () => true });
    expect(actions.toggleLayers).toHaveBeenCalledTimes(1);
    handle.destroy();
    press("f");
    expect(actions.toggleLayers).toHaveBeenCalledTimes(1); // listener removed
  });

  test("P10 frameRateMonitor factory degrades without a viewer surface", async () => {
    const m = (await import(
      "gev-engine/src/ui/frameRateMonitor.js"
    )) as Record<string, any>;
    expect(typeof m.createFrameRateMonitor).toBe("function");
    // ({ viewer, documentRef = document })
    expect(m.createFrameRateMonitor.length).toBe(1);
    // Contract: a missing #title-bar host or a Cesium-less viewer yields a
    // destroy-only no-op — never a throw, so callers can mount unguarded.
    const noop = m.createFrameRateMonitor({ viewer: {} });
    expect(typeof noop.destroy).toBe("function");
    expect(() => noop.destroy()).not.toThrow();
  });

  test("P10 scenePolicy tracking-strip + exclusivity decisions are pinned", async () => {
    const m = (await import(
      "gev-engine/src/scenes/scenePolicy.js"
    )) as Record<string, any>;
    expect(m.SCENE_TRACKING_PARAM_KEYS).toEqual([
      "selectedFlightsTrackingId",
      "selectedMilitaryTrackingId",
      "selectedSatTrackingId",
    ]);
    expect(m.SCENE_KEPT_SELECTION_PARAM_KEYS).toEqual(["selectedCameraId"]);
    expect(m.SCENE_SELECTION_PARAM_PATTERN).toBeInstanceOf(RegExp);
    expect(m.SCENE_EXCLUSIVITY_PROBE_LAYER_ID).toBe(
      "__scene-exclusivity-probe__",
    );
    for (const [name, arity] of [
      ["stripSceneTrackingParams", 1],
      ["sceneRequiresContextModeExit", 1],
      ["sceneLayerPlan", 2],
    ] as const) {
      expect(typeof m[name], `scenePolicy.${name}`).toBe("function");
      expect(m[name].length, `scenePolicy.${name} arity`).toBe(arity);
    }
    // Tracking params are stripped; a params bag that was ONLY tracking
    // dissolves to undefined (never an empty object pushed at the layer).
    expect(
      m.stripSceneTrackingParams({ selectedFlightsTrackingId: "x", opacity: 1 }),
    ).toEqual({ opacity: 1 });
    expect(m.stripSceneTrackingParams({ selectedSatTrackingId: "x" })).toBeUndefined();
    expect(m.stripSceneTrackingParams(undefined)).toBeUndefined();
    // No active context mode → no forced exit.
    expect(m.sceneRequiresContextModeExit(null)).toBe(false);
    // Only declared + registered layers are planned, in declaration order.
    expect(
      m.sceneLayerPlan(
        {
          flights: {
            enabled: true,
            params: { selectedFlightsTrackingId: "t", zoom: 3 },
          },
          ghost: { enabled: true },
        },
        new Set(["flights"]),
      ),
    ).toEqual([{ id: "flights", enabled: true, params: { zoom: 3 } }]);
  });

  test("P10 services/application slot registry + configure guard are pinned", async () => {
    const m = (await import(
      "gev-engine/src/services/application.js"
    )) as Record<string, any>;
    expect(Object.isFrozen(m.applicationServices)).toBe(true);
    expect(Object.keys(m.applicationServices).sort()).toEqual([
      "boundaries",
      "features",
      "regional",
      "summary",
      "terrain",
      "weather",
    ]);
    expect(typeof m.configureApplicationServices).toBe("function");
    expect(m.configureApplicationServices.length).toBe(1);
    // Unknown slot names are contract misuse, never silently ignored.
    expect(() =>
      m.configureApplicationServices({ bogusService: () => {} }),
    ).toThrow(/Unknown application service: bogusService/);
    const release = m.configureApplicationServices({
      boundaries: { query: async () => ({}) },
    });
    expect(typeof release).toBe("function");
    release();
  });

  test("P10 localStorage namespace: vendor godsEyeView.<version> panel keys stay pinned", () => {
    const pos = readVendor("src/ui/panelPositionControls.js");
    // Versioned namespace prefix — layout/collapse generations.
    expect(pos).toMatch(/const PANEL_LAYOUT_STORAGE_VERSION = 'v6';/);
    expect(pos).toMatch(/const PANEL_POSITION_STORAGE_VERSION = 'v8';/);
    // Position + collapsed key templates (godsEyeView.<ver>.panel*.<panelId>).
    expect(pos).toContain(
      "`godsEyeView.${PANEL_POSITION_STORAGE_VERSION}.panelPos.${panelId}`",
    );
    expect(pos).toContain(
      "`godsEyeView.${PANEL_LAYOUT_STORAGE_VERSION}.panelCollapsed.${panelId}`",
    );
    // Reset marker + the legacy-prefix sweep the migration toast depends on.
    expect(pos).toContain(
      "`godsEyeView.${PANEL_POSITION_STORAGE_VERSION}.layoutResetNotified`",
    );
    expect(pos).toContain("'godsEyeView.v6.panelPos.'");
    // NOTE: the plan's `godsEyeView.v6.layoutRightPanels` literal never existed
    // — panelLayoutController.js owns no storage key. The second panel storage
    // family is `panelCollapsed` (pinned above), so that is the real seam this
    // guard protects; an upstream rename of either template trips here.
    expect(pos).toContain("panelCollapsed");
  });
});

// ── c2: engine behavior contracts (mock fetch, no network) ─────────────────

describe("c2: engine behavior contracts (mock fetch, zero network)", () => {
  const jsonFetch =
    (body: unknown, status = 200): ApiFetch =>
    async () =>
      new Response(JSON.stringify(body), { status });

  test("envelope stubs resolve degraded-by-contract: 503, empty records, unknown freshness", async () => {
    // What the engine reads as "degraded" (flights/ingestion.js:41: stale ||
    // freshness === "unknown") must be exactly what an idle stub produces.
    const s = createIntelHubLayerSources({ apiFetch: jsonFetch({}) });
    const env = await (s as Record<string, any>).vessels.getSnapshot();
    expect(env.records).toEqual([]);
    expect(env.status).toBe(503);
    expect(env.freshness).toBe("unknown");
    expect(env.observedAtMs).toBeNull();
    expect(env.ageMs).toBeNull();
    // FIRMS has its own payload shape: empty + stale, never fabricated fires.
    const firms = await (s as Record<string, any>).firms.getSnapshot();
    expect(firms).toMatchObject({ fires: [], stale: true });
  });

  test("satellites: transport failure resolves {ok:false,status} and never throws; unknown group is a contract-misuse TypeError", async () => {
    const s = createIntelHubLayerSources({
      apiFetch: async () => new Response("upstream boom", { status: 500 }),
    });
    // The engine maps non-ok to an empty group (satellites/ingestion.js:34),
    // so the adapter resolving — not rejecting — on HTTP 500 is the contract.
    await expect(
      (s as Record<string, any>).satellites.readGroup("visual"),
    ).resolves.toEqual({ ok: false, status: 500, text: "" });
    // A group outside the served set is a programming error, not a transport
    // state: TypeError, per T5's source contract.
    await expect(
      (s as Record<string, any>).satellites.readGroup("not-a-real-group"),
    ).rejects.toBeInstanceOf(TypeError);
  });

  test("earthquakes: non-array body degrades to [] (never reaches `for...of` throw); non-ok rejects with the status", async () => {
    const s = createIntelHubLayerSources({
      apiFetch: jsonFetch({ error: "proxy html page" }),
    });
    await expect(
      (s as Record<string, any>).earthquakes.getSnapshot(),
    ).resolves.toEqual([]);
    const s502 = createIntelHubLayerSources({
      apiFetch: jsonFetch({ error: "x" }, 502),
    });
    await expect(
      (s502 as Record<string, any>).earthquakes.getSnapshot(),
    ).rejects.toThrow("IntelHub earthquakes HTTP 502");
  });

  test("flights/military: hub degraded body resolves stale/503 with null epoch; non-ok rejects with the status", async () => {
    const s = createIntelHubLayerSources({
      apiFetch: jsonFetch({ stale: true, aircraft: [] }),
    });
    for (const layer of ["flights", "military"]) {
      const env = await (s as Record<string, any>)[layer].getSnapshot();
      expect(env.records).toEqual([]);
      expect(env.stale).toBe(true);
      // 503 on the hub's degraded body is intentional (T6 M-3) — the assertion
      // is the degraded semantics, not a universal 200.
      expect(env.status).toBe(503);
      expect(env.observedAtMs).toBeNull();
      expect(env.ageMs).toBeNull();
    }
    const s401 = createIntelHubLayerSources({
      apiFetch: jsonFetch({ error: "unauthorized" }, 401),
    });
    await expect(
      (s401 as Record<string, any>).flights.getSnapshot(),
    ).rejects.toThrow("HTTP 401");
    await expect(
      (s401 as Record<string, any>).military.getSnapshot(),
    ).rejects.toThrow("HTTP 401");
  });

  test("transit stub keeps the engine out of its unauthenticated fallback: 503 + {vehicles:[]} + empty history", async () => {
    // T3 I-2: without a sources.transit entry the engine falls back to its own
    // same-origin UNauthenticated /api/transit polling. The stub's 503 +
    // empty vehicles lands the layer in its ordinary "temporarily
    // unavailable" state instead.
    const s = createIntelHubLayerSources({ apiFetch: jsonFetch({}) });
    const snap = await (s as Record<string, any>).transit.requestSnapshot();
    expect(snap).toMatchObject({ ok: false, status: 503 });
    await expect(snap.json()).resolves.toEqual({ vehicles: [] });
    await expect(
      (s as Record<string, any>).transit.getHistory(),
    ).resolves.toEqual({ epochs: [] });
  });
});

// ── c3: vendor boundary (UPSTREAM.json exceptions are the single source) ───

describe("c3: vendor boundary (UPSTREAM.json exceptions)", () => {
  const upstreamMeta = JSON.parse(readVendor("UPSTREAM.json")) as {
    exceptions?: Array<string | { file: string; reason?: string }>;
  };
  const exceptionFiles = new Set(
    (upstreamMeta.exceptions ?? []).map((e) => (typeof e === "string" ? e : e.file)),
  );

  test("the vite.config.ts local_data externalize is a DECLARED exception, not an implicit one", () => {
    expect(
      exceptionFiles.has("console/vite.config.ts"),
      "vite.config.ts must be listed in UPSTREAM.json exceptions " +
        "(T8-reviewed intentional local_data externalize)",
    ).toBe(true);
    expect(existsSync(join(consoleRoot, "vite.config.ts"))).toBe(true);
  });

  test("no host file references the vendor's local_data outside declared exceptions", () => {
    const hits: string[] = [];
    const scan = (dir: string) => {
      for (const entry of readdirSync(dir)) {
        if (entry === "node_modules" || entry === "dist" || entry.startsWith("."))
          continue;
        const full = join(dir, entry);
        if (statSync(full).isDirectory()) {
          scan(full);
          continue;
        }
        if (!/\.(ts|tsx|js|mts|cts|json)$/.test(entry)) continue;
        // The guard file itself is the scanner — its own literal patterns are
        // not host references to vendor internals.
        if (full === fileURLToPath(import.meta.url)) continue;
        if (/local_data/.test(readFileSync(full, "utf8")))
          hits.push(relative(consoleRoot, full));
      }
    };
    scan(join(consoleRoot, "src"));
    for (const cfg of ["vite.config.ts", "vitest.config.ts", "tsconfig.json"]) {
      const full = join(consoleRoot, cfg);
      if (existsSync(full) && /local_data/.test(readFileSync(full, "utf8")))
        hits.push(cfg);
    }
    expect(hits.length).toBeGreaterThan(0); // sanity: the exemption is exercised
    for (const hit of hits)
      expect(
        exceptionFiles.has(`console/${hit}`),
        `${hit} references local_data — declare it in UPSTREAM.json ` +
          "exceptions or remove the reference",
      ).toBe(true);
  });

  test("the externalize plugin itself is still wired in vite.config.ts", () => {
    const vite = readConsole("vite.config.ts");
    expect(vite).toContain("gev-local-data-external");
    // The resolveId pattern — externalizing the un-vendored local_data packs
    // is what keeps `npm run build` green without touching the vendor tree.
    expect(vite).toContain("local_data\\/[^/]+\\/[^/]+\\.json$");
    expect(vite).toContain("external: true");
  });

  test("engine alias channel stays in sync across vite / vitest / tsconfig (Ruling 6)", () => {
    // Ruling 6's stated cost — "三处配置需保持同步" — is enforced here:
    // vitest ignores vite.config.ts entirely when vitest.config.ts exists, so
    // a config that drops the alias breaks tests keyless/broken while the
    // build keeps working.
    const vite = readConsole("vite.config.ts");
    const vitest = readConsole("vitest.config.ts");
    const tsconfig = readConsole("tsconfig.json");
    expect(vite).toMatch(/alias:\s*\{[\s\S]*?"gev-engine"/);
    expect(vitest).toMatch(/alias:\s*\{[\s\S]*?"gev-engine"/);
    expect(tsconfig).toMatch(/"gev-engine\/\*"/);
  });

  test("every declared exception file exists on disk", () => {
    for (const file of exceptionFiles)
      expect(existsSync(join(consoleRoot, "..", file)), file).toBe(true);
  });
});
