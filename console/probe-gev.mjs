#!/usr/bin/env node
// probe-gev.mjs — headless smoke for the GEV-engine /globe page (GlobeV2).
//
// Why a new file (T14): the retired P1 probe asserted the self-built 2D globe
// (`[data-probe="aircraft-count"]`, `document.querySelector("canvas")`). The
// /globe route now mounts GlobeV2 (T8) with an engine-owned Cesium canvas, and
// the P1 page + probe are retired in T16 — the old assertions would have
// silently "passed" against a page they no longer describe. This file replaced
// the P1 `probe-globe.mjs` (a stand-in pass-through during T14/T15, now
// deleted): run `probe-gev.mjs` directly.
//
// Asserted (T8-T11 DOM contract + engine boot order):
//   [data-hud="top|left|right|bottom"]   four-edge HUD frame, each edge
//                                        individually (T8)
//   #cesiumContainer canvas              engine viewer mounted + non-zero size
//   [data-testid="hud-top-bar"]          T11 top bar
//   [data-testid="hud-bottom-bar"]       T11 bottom bar
//   [data-testid="hud-detail-panel"]     T10 right detail panel
//   .hud-detail-empty                    T10 empty state: present + non-empty
//                                        whenever the panel kind is "none"
//   [data-testid="hud-layer-rail"]       T9 rail — rendered only after the
//                                        engine's start() promise resolved, so
//                                        it doubles as a boot-order assertion
//                                        (scene → data phase → HUD rail)
//   [data-testid="hud-layer-rail"]
//     .hud-rail-icons button             T9 domain icons, not just the
//                                        always-present collapse handle
//   [data-testid="hud-layer-count"]      numeric "n/m" ⇒ dataManager attached
//   #loading-screen>.loader-status       text must not start with "Error" —
//                                        engine failure path (gev-engine/src/
//                                        main.js:13) writes "Error: …" here
//   #loading-screen height               < viewport height: a bottom-right
//                                        status chip (T8/HudFrame.tsx:33 +
//                                        globe-hud/hud.css:18), NOT a
//                                        full-screen boot overlay
//   REST listKey + envelope              GET /api/v1/globe/{aircraft,satellites}
//                                        must be 2xx JSON AND carry the
//                                        declared list key (`aircraft`/`items`)
//                                        as an array, plus at least one of the
//                                        endpoint's envelope fields (aircraft:
//                                        coverage|stale) — shape drift WARNs

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
// wire, the HUD's build-time basemap label, the loader text itself (only its
// "Error" prefix is gated), the loader chip size, the rail-icon count, and the
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
let p3CountBefore = null;
let p3CountAfter = null;
let p3Toggled = [];
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
      railIcons: document.querySelectorAll(
        '[data-testid="hud-layer-rail"] .hud-rail-icons button',
      ).length,
      detailEmpty: q(".hud-detail-empty"),
      detailEmptyText: text(".hud-detail-empty"),
      detailCollapsed:
        document
          .querySelector('[data-testid="hud-detail-panel"]')
          ?.classList.contains("collapsed") ?? null,
      loaderScreenPresent: !!document.getElementById("loading-screen"),
      loaderScreenHeight:
        document.getElementById("loading-screen")?.getBoundingClientRect()
          .height ?? null,
      viewportHeight: window.innerHeight,
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

  const readCount = async (path, listKey, atLeastOneOf = []) => {
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
      // Envelope-shape drift is reported, never fatal (the values are data, the
      // shape is code) — but silently reading a renamed key as "0 rows" is how
      // a broken API reads as a quiet zero.
      const list = Array.isArray(res.body[listKey]) ? res.body[listKey] : null;
      if (list === null)
        warnings.push(
          `REST ${path} → "${listKey}" is not an array (envelope shape changed?)`,
        );
      if (typeof res.body.count !== "number")
        warnings.push(`REST ${path} → "count" is not numeric`);
      // A healthy envelope carries at least one of `atLeastOneOf`. For aircraft
      // that is `coverage` (live adsb/opensky merge) or `stale:true` (both
      // snapshots missing — adsb.rs:211); the fresh path deliberately has no
      // `stale` key, so demanding it per-endpoint would WARN on every healthy
      // run. Satellites declares neither field (count+items only).
      if (
        atLeastOneOf.length &&
        !atLeastOneOf.some((field) => field in res.body)
      )
        warnings.push(
          `REST ${path} → none of [${atLeastOneOf.join(", ")}] present (envelope shape changed?)`,
        );
      return {
        count:
          typeof res.body.count === "number"
            ? res.body.count
            : (list?.length ?? 0),
        // When `count` was absent we fall back to the list length — don't let
        // that inference masquerade as "the first adsb rotation hasn't run".
        countInferred: typeof res.body.count !== "number",
        stale: res.body.stale === true,
        coverage: typeof res.body.coverage === "string" ? res.body.coverage : null,
      };
    } catch (e) {
      failures.push(`REST ${path} → ${String(e)}`);
      return { error: String(e) };
    }
  };

  aircraft = await readCount("/api/v1/globe/aircraft", "aircraft", [
    "coverage",
    "stale",
  ]);
  satellites = await readCount("/api/v1/globe/satellites", "items");

  // ---- GEV P3 T16: exercise the newly-wired layers through the rail ------
  // vessels (sea), cctv + traffic (ground), installations (infra). Flyout
  // labels come from upstream catalog metadata (may rename), so we toggle by
  // domain: open each domain's flyout and switch every currently-OFF layer
  // ON, then assert the n/m layer counter moved and no pageerror fired.
  // Stubs enable-empty by contract, so this also smoke-tests them. All
  // toggles are restored OFF afterwards. (traffic with no TomTom key takes
  // the engine's simulated path — zero tile-quota burn.)
  const readLayerCount = async () =>
    page.evaluate(() => {
      const el = document.querySelector('[data-testid="hud-layer-count"]');
      const m = el?.textContent?.match(/(\d+)\s*\/\s*(\d+)/);
      return m ? Number(m[1]) : null;
    });
  p3CountBefore = await readLayerCount();
  p3Toggled = [];
  for (const [domain, zh] of [
    ["sea", "海洋"],
    ["ground", "地面"],
    ["infra", "基建"],
  ]) {
    const icon = page.locator(
      `[data-testid="hud-layer-rail"] .hud-rail-icons button[aria-label*="${zh}"]`,
    );
    if (!(await icon.count())) {
      warnings.push(`P3 rail: ${domain} icon missing`);
      continue;
    }
    await icon.first().click();
    const flyout = page.locator('.hud-rail-flyout');
    await flyout.waitFor({ state: "visible", timeout: 5000 }).catch(() => {});
    const boxes = flyout.locator('input[type="checkbox"]');
    const n = await boxes.count();
    for (let i = 0; i < n; i++) {
      const box = boxes.nth(i);
      if (!(await box.isChecked())) {
        await box
          .check({ timeout: 3000 })
          .catch((e) => warnings.push(`P3 toggle ${domain}[${i}]: ${String(e).slice(0, 80)}`));
        p3Toggled.push(`${domain}[${i}]`);
      }
    }
    await page.keyboard.press("Escape").catch(() => {});
  }
  // Let the LayerLifecycle serial queues settle + first fetches land.
  await page.waitForTimeout(6000);
  p3CountAfter = await readLayerCount();
  // Restore: uncheck everything we turned on (same domain loop, reversed).
  for (const [domain, zh] of [
    ["infra", "基建"],
    ["ground", "地面"],
    ["sea", "海洋"],
  ]) {
    const icon = page.locator(
      `[data-testid="hud-layer-rail"] .hud-rail-icons button[aria-label*="${zh}"]`,
    );
    if (!(await icon.count())) continue;
    await icon.first().click();
    const flyout = page.locator('.hud-rail-flyout');
    await flyout.waitFor({ state: "visible", timeout: 5000 }).catch(() => {});
    const boxes = flyout.locator('input[type="checkbox"]');
    const n = await boxes.count();
    for (let i = n - 1; i >= 0; i--) {
      const box = boxes.nth(i);
      if (p3Toggled.includes(`${domain}[${i}]`) && (await box.isChecked()))
        await box.uncheck({ timeout: 3000 }).catch(() => {});
    }
    await page.keyboard.press("Escape").catch(() => {});
  }
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
  // Four-edge frame: every edge is a gate (the brief's "all four true").
  for (const [edge, present] of Object.entries(snap.hud))
    if (!present) failures.push(`[data-hud="${edge}"] missing`);
  if (!snap.topBar) failures.push('[data-testid="hud-top-bar"] missing');
  if (!snap.bottomBar) failures.push('[data-testid="hud-bottom-bar"] missing');
  if (!snap.detailPanel) failures.push('[data-testid="hud-detail-panel"] missing');
  if (snap.fatal) failures.push(`.hud-fatal shown: ${snap.fatal}`);
  // T9: the collapse handle exists in both states, so "rail present" says
  // nothing about the domain icons — count them.
  if (!snap.railIcons)
    failures.push(
      '[data-testid="hud-layer-rail"] has no .hud-rail-icons button (collapsed rail or missing domain icons)',
    );
  // T10 empty state: kind=none must be the placeholder, not a blank panel.
  if (snap.detailKind === "none" && snap.detailCollapsed !== true) {
    if (!snap.detailEmpty)
      failures.push(
        '.hud-detail-empty missing (detail kind=none, panel not collapsed)',
      );
    else if (!snap.detailEmptyText)
      failures.push(".hud-detail-empty is empty");
  }
  // Loader semantics (I2): #loading-screen is the resident bottom-right status
  // chip, not a boot overlay — assert both halves of that meaning.
  if (!snap.loaderScreenPresent) failures.push("#loading-screen missing");
  else if (
    typeof snap.loaderScreenHeight === "number" &&
    snap.loaderScreenHeight >= snap.viewportHeight
  )
    failures.push(
      `#loading-screen is a full-screen overlay (${Math.round(snap.loaderScreenHeight)}px >= viewport ${snap.viewportHeight}px)`,
    );
  else if (snap.loaderScreenHeight === 0)
    warnings.push("#loading-screen has zero height (hidden status chip?)");
  if (snap.loader === null)
    failures.push("#loading-screen .loader-status missing");
  else if (/^error/i.test(snap.loader))
    failures.push(`loader-status reports an engine error: ${snap.loader}`);
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
  else if (aircraft.count === 0 && !aircraft.countInferred)
    warnings.push("aircraft count is 0 (first adsb rotation may not have finished)");
}
if (satellites && !satellites.error && satellites.count === 0)
  warnings.push("satellites count is 0 (celestrak first sweep pending?)");

// GEV P3 T16: the rail exercise must have found off-layers in the three
// domains and enabling them must move the n/m counter (a stuck counter means
// the LayerLifecycle never engaged — exactly what this probe exists for).
if (p3Toggled.length === 0)
  failures.push("P3 rail exercise toggled 0 layers (sea/ground/infra flyouts empty or rail inert)");
else if (
  p3CountBefore !== null &&
  p3CountAfter !== null &&
  p3CountAfter <= p3CountBefore
)
  failures.push(
    `P3 rail exercise: layer count did not increase (${p3CountBefore} → ${p3CountAfter} after ${p3Toggled.length} toggles)`,
  );
if (snap && snap.webgl === "none")
  warnings.push("#cesiumContainer canvas has no WebGL context (SwiftShader missing?)");

const hudParts = snap
  ? ["top", "left", "right", "bottom"].filter((edge) => snap.hud[edge])
  : [];
// hudOk is exactly `hudParts.length === 4`, and the failures loop above makes
// every false edge fatal — the printed flag and the exit code cannot diverge.
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
    `loader-screen=${
      snap.loaderScreenHeight === null
        ? "-"
        : `${Math.round(snap.loaderScreenHeight)}px/${snap.viewportHeight}px`
    } rail-icons=${snap.railIcons}`,
  );
  console.log(
    `detail-kind=${snap.detailKind ?? "-"} detail-empty=${
      snap.detailEmpty ? (snap.detailEmptyText ? "ok" : "blank") : "-"
    } aircraft-coverage=${aircraft?.coverage ?? "-"}`,
  );
  console.log(
    `p3-layers=${p3Toggled.length} toggled, count ${p3CountBefore ?? "-"} → ${p3CountAfter ?? "-"}`,
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
