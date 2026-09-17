#!/usr/bin/env node
// probe-gev.mjs — headless smoke for the GEV-engine /globe page (GlobeV2).
//
// Why a new file (T14): probe-globe.mjs asserted the P1 self-built 2D globe
// (`[data-probe="aircraft-count"]`, `document.querySelector("canvas")`). The
// /globe route now mounts GlobeV2 (T8) with an engine-owned Cesium canvas, and
// the P1 page is retired in T16 — the old assertions would have silently
// "passed" against a page they no longer describe. probe-globe.mjs is kept as a
// one-line delegator to this file and deleted in T16.
//
// Asserted (T8-T11 DOM contract + engine boot order):
//   [data-hud="top|left|right|bottom"]   four-edge HUD frame (T8)
//   #cesiumContainer canvas              engine viewer mounted + non-zero size
//   [data-testid="hud-top-bar"]          T11 top bar
//   [data-testid="hud-bottom-bar"]       T11 bottom bar
//   [data-testid="hud-detail-panel"]     T10 right detail panel
//   [data-testid="hud-layer-rail"]       T9 rail — rendered only after the
//                                        engine's start() promise resolved, so
//                                        it doubles as a boot-order assertion
//                                        (scene → data phase → HUD rail)
//   [data-testid="hud-layer-count"]      numeric "n/m" ⇒ dataManager attached
//
// NOT asserted: `window.__godsEyeView`. The engine's tools phase is deliberately
// stubbed in gev-boot/application.ts (IntelHub's HUD owns the chrome), so the
// debug handle never exists — the layer readout is taken from the DOM instead.
//
// Counts: read from the same REST endpoints the layers consume (never upstream
// direct) — GET /api/v1/globe/aircraft (adsb + OpenSky read-time merge) and
// GET /api/v1/globe/satellites (PG TLE catalog). Non-2xx / non-JSON fails the
// probe (transport + auth are code properties); the values themselves are only
// reported (0 is a first-rotation timing artifact, not a defect).
//
// Report-only (never fails): which basemap provider actually answered on the
// wire, the HUD's build-time basemap label, the engine loader text, and the
// WebGL context kind. The Google → Cesium ion → Esri fallback is a deployment
// property (which keys the VM build injected), not a code property.
//
// Usage: node console/probe-gev.mjs <base-url> <console-api-key>
// Exit:  0 ok · 1 assertion failed · 2 usage
import { chromium } from "playwright";

const [rawBase, key] = process.argv.slice(2);
if (!rawBase || !key) {
  console.error("usage: probe-gev.mjs <base-url> <console-api-key>");
  process.exit(2);
}
const BASE = rawBase.replace(/\/+$/, "");

// Separate budgets per boot stage: a scene failure must report as "canvas
// missing", not as "HUD missing". The lazy Globe chunk carries Cesium (~6 MB),
// hence the generous mount budget.
const MOUNT_TIMEOUT_MS = 60_000;
const SCENE_TIMEOUT_MS = 45_000;
const DATA_TIMEOUT_MS = 30_000;
const REST_TIMEOUT_MS = 30_000;

const pageErrors = [];
const engineNotes = [];
const wire = new Set();

const browser = await chromium.launch();
const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
await ctx.addInitScript(
  (k) => localStorage.setItem("intelhub.console.key", k),
  key,
);
const page = await ctx.newPage();
page.on("pageerror", (e) => pageErrors.push(String(e)));
// Engine boot notes (scene.js logs the photoreal route / the fallback reason).
page.on("console", (msg) => {
  const text = msg.text();
  if (/Google 3D Tiles/i.test(text)) engineNotes.push(text.slice(0, 240));
});
// Basemap classification from the wire — the HUD label is derived from
// build-time key presence (T11), the requests show what the engine reached for.
page.on("request", (req) => {
  const url = req.url();
  if (/tile\.googleapis\.com|photorealistic3dtiles/i.test(url))
    wire.add("google-photoreal");
  else if (/ion\.cesium\.com/i.test(url)) wire.add("cesium-ion");
  else if (/arcgisonline\.com/i.test(url)) wire.add("esri-imagery");
});

const missing = [];
const failures = [];
const warnings = [];
const gate = async (selector, timeoutMs, label) => {
  try {
    await page.waitForSelector(selector, {
      timeout: timeoutMs,
      state: "attached",
    });
    return true;
  } catch {
    missing.push(label);
    return false;
  }
};

let snap = null;
let aircraft = null;
let satellites = null;
let bodySnippet = null;

try {
  await page.goto(`${BASE}/globe`, { waitUntil: "load", timeout: 60_000 });

  // 1. React mounted (lazy Globe chunk + first paint).
  const mounted = await gate(".hud-root", MOUNT_TIMEOUT_MS, ".hud-root (React mount)");
  if (mounted) {
    // 2. Engine scene booted (Cesium viewer + WebGL canvas).
    await gate(
      "#cesiumContainer canvas",
      SCENE_TIMEOUT_MS,
      "#cesiumContainer canvas (engine scene)",
    );
    // 3. Data phase resolved: the T9 rail only mounts once start() resolved
    //    AND the dataManager exists.
    await gate(
      '[data-testid="hud-layer-rail"]',
      DATA_TIMEOUT_MS,
      '[data-testid="hud-layer-rail"] (engine data phase)',
    );
    const layerCountNumeric = await page
      .waitForFunction(
        () => {
          const el = document.querySelector('[data-testid="hud-layer-count"]');
          return !!el && /\d\s*\/\s*\d/.test(el.textContent ?? "");
        },
        null,
        { timeout: DATA_TIMEOUT_MS },
      )
      .then(() => true)
      .catch(() => false);
    if (!layerCountNumeric)
      missing.push('[data-testid="hud-layer-count"] numeric (dataManager attached)');
  }

  snap = await page.evaluate(() => {
    const q = (selector) => !!document.querySelector(selector);
    const text = (selector) =>
      document.querySelector(selector)?.textContent?.trim() ?? null;
    const field = (selector) =>
      document.querySelector(selector)?.getAttribute("data-kind") ?? null;
    const canvas = document.querySelector("#cesiumContainer canvas");
    // getContext with a type other than the live one returns null (never
    // creates a second context), so this reads the Cesium context back.
    let webgl = null;
    if (canvas) {
      try {
        webgl = canvas.getContext("webgl2")
          ? "webgl2"
          : canvas.getContext("webgl")
            ? "webgl"
            : "none";
      } catch {
        webgl = "error";
      }
    }
    return {
      hud: {
        top: q('[data-hud="top"]'),
        left: q('[data-hud="left"]'),
        right: q('[data-hud="right"]'),
        bottom: q('[data-hud="bottom"]'),
      },
      canvasPresent: !!canvas,
      canvas: !!canvas && canvas.width > 0 && canvas.height > 0,
      canvasSize: canvas ? `${canvas.width}x${canvas.height}` : null,
      webgl,
      rootChildren: document.getElementById("root")?.childElementCount ?? 0,
      topBar: q('[data-testid="hud-top-bar"]'),
      bottomBar: q('[data-testid="hud-bottom-bar"]'),
      detailPanel: q('[data-testid="hud-detail-panel"]'),
      rail: q('[data-testid="hud-layer-rail"]'),
      clock: text('[data-testid="hud-utc-clock"]'),
      layerCount: text('[data-testid="hud-layer-count"]'),
      basemapLabel: text('[data-testid="hud-basemap-style"]'),
      detailKind: field('[data-testid="hud-detail-panel"]'),
      fatal: text(".hud-fatal"),
      loader: text("#loading-screen .loader-status"),
    };
  });

  // Diagnostics for the failure path (>300 chars, whitespace-collapsed): with
  // an ErrorBoundary in App.tsx a chunk/render failure shows as text, not as an
  // empty #root.
  bodySnippet = await page
    .evaluate(() =>
      (document.body?.innerText || "").replace(/\s+/g, " ").slice(0, 300),
    )
    .catch(() => null);

  const readJson = async (path) => {
    const res = await page.request.get(`${BASE}${path}`, {
      headers: { Authorization: `Bearer ${key}` },
      timeout: REST_TIMEOUT_MS,
    });
    const raw = await res.text();
    let body = null;
    try {
      body = JSON.parse(raw);
    } catch {
      body = null;
    }
    return {
      status: res.status(),
      ok: res.ok(),
      body,
      // One-line snippet: a Vite/dev or proxy fallback answers 200 with the SPA
      // index.html, which must read as "not the JSON API" at a glance.
      snippet: raw.replace(/\s+/g, " ").slice(0, 160),
    };
  };

  const readCount = async (path, listKey) => {
    try {
      const res = await readJson(path);
      if (!res.ok || res.body === null) {
        const detail =
          res.body === null
            ? `non-JSON body: ${res.snippet || "<empty>"}`
            : `HTTP ${res.status}`;
        failures.push(`REST ${path} → ${detail}`);
        return { error: detail };
      }
      const list = Array.isArray(res.body[listKey]) ? res.body[listKey] : [];
      return {
        count: typeof res.body.count === "number" ? res.body.count : list.length,
        stale: res.body.stale === true,
        coverage: typeof res.body.coverage === "string" ? res.body.coverage : null,
      };
    } catch (e) {
      failures.push(`REST ${path} → ${String(e)}`);
      return { error: String(e) };
    }
  };

  aircraft = await readCount("/api/v1/globe/aircraft", "aircraft");
  satellites = await readCount("/api/v1/globe/satellites", "items");
} catch (e) {
  failures.push(`probe crashed: ${String(e)}`);
} finally {
  await browser.close().catch(() => {});
}

// ---- assertions ------------------------------------------------------------
for (const item of missing) failures.push(`missing ${item}`);
if (snap) {
  if (!snap.canvasPresent) failures.push("#cesiumContainer has no canvas child");
  else if (!snap.canvas) failures.push("#cesiumContainer canvas is zero-sized");
  if (!snap.rootChildren) failures.push("#root is empty (React root unmounted)");
  if (!snap.topBar) failures.push('[data-testid="hud-top-bar"] missing');
  if (!snap.bottomBar) failures.push('[data-testid="hud-bottom-bar"] missing');
  if (!snap.detailPanel) failures.push('[data-testid="hud-detail-panel"] missing');
  if (snap.fatal) failures.push(`.hud-fatal shown: ${snap.fatal}`);
}

// Keep the P1 pageerror filter narrowed to the two known-benign browser/engine
// emissions (ResizeObserver loop warnings, Cesium GroupMarkerNotSet).
const fatalErrors = pageErrors.filter(
  (e) => !/ResizeObserver|GroupMarkerNotSet/i.test(e),
);
if (fatalErrors.length) failures.push(`${fatalErrors.length} pageerror(s)`);

if (aircraft && !aircraft.error) {
  if (aircraft.stale)
    warnings.push(
      `aircraft snapshot is stale (${aircraft.count} rows) — no fresh adsb/opensky snapshot`,
    );
  else if (aircraft.count === 0)
    warnings.push("aircraft count is 0 (first adsb rotation may not have finished)");
}
if (satellites && !satellites.error && satellites.count === 0)
  warnings.push("satellites count is 0 (celestrak first sweep pending?)");
if (snap && snap.webgl === "none")
  warnings.push("#cesiumContainer canvas has no WebGL context (SwiftShader missing?)");

const hudParts = snap
  ? ["top", "left", "right", "bottom"].filter((edge) => snap.hud[edge])
  : [];
const hudOk = hudParts.length === 4;

// ---- report ---------------------------------------------------------------
console.log(
  `hud=${hudOk} canvas=${snap ? snap.canvas : false} aircraft=${
    aircraft && !aircraft.error ? aircraft.count : "n/a"
  } satellites=${satellites && !satellites.error ? satellites.count : "n/a"} pageerrors=${fatalErrors.length}`,
);
if (snap) {
  console.log(
    `hud-parts=${hudParts.join(",") || "-"} canvas-size=${snap.canvasSize ?? "-"} webgl=${snap.webgl ?? "-"} layers=${snap.layerCount ?? "-"} clock=${snap.clock ?? "-"}`,
  );
  console.log(
    `basemap-label=${snap.basemapLabel ?? "-"} basemap-observed=${[...wire].join("+") || "-"} loader=${JSON.stringify(snap.loader ?? "-")}`,
  );
  console.log(
    `detail-kind=${snap.detailKind ?? "-"} aircraft-coverage=${aircraft?.coverage ?? "-"}`,
  );
}
for (const note of engineNotes) console.log(`engine-note=${note}`);
for (const warning of warnings) console.error(`WARN ${warning}`);

if (failures.length) {
  console.error(`probe-gev FAILED (${failures.length})`);
  for (const failure of failures) console.error(`  ${failure}`);
  if (fatalErrors.length) console.error(fatalErrors.join("\n"));
  if (bodySnippet) console.error(`  body: ${bodySnippet}`);
  process.exit(1);
}
console.log("probe-gev OK");
