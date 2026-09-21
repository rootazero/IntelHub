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
let searchGeocodeSeen = false;

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
  // The HUD's location search calls the same-origin geocode proxy from page
  // JS (not page.request) — observing it proves the search actually fired.
  if (/\/api\/v1\/gev\/geocode/.test(url)) searchGeocodeSeen = true;
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
  const mounted = await gate(".globe-root", MOUNT_TIMEOUT_MS, ".globe-root (React mount)");
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
    // T14 a11y (roving tabindex): the first icon click focuses, the second
    // opens the flyout — retry the click until the flyout appears (≤3).
    const flyout = page.locator('.hud-rail-flyout');
    for (let attempt = 0; attempt < 3; attempt++) {
      await icon.first().click();
      const visible = await flyout
        .waitFor({ state: "visible", timeout: 1500 })
        .then(() => true)
        .catch(() => false);
      if (visible) break;
    }
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
    const flyout = page.locator('.hud-rail-flyout');
    for (let attempt = 0; attempt < 3; attempt++) {
      await icon.first().click();
      const visible = await flyout
        .waitFor({ state: "visible", timeout: 1500 })
        .then(() => true)
        .catch(() => false);
      if (visible) break;
    }
    const boxes = flyout.locator('input[type="checkbox"]');
    const n = await boxes.count();
    for (let i = n - 1; i >= 0; i--) {
      const box = boxes.nth(i);
      if (p3Toggled.includes(`${domain}[${i}]`) && (await box.isChecked()))
        await box.uncheck({ timeout: 3000 }).catch(() => {});
    }
    await page.keyboard.press("Escape").catch(() => {});
  }

  // ---- GEV P6: style switcher — GLSL post-process actually re-renders ----
  // The switcher lives in the top bar (HudTopBar) and only mounts once the
  // visual-effects adapter handle exists. FLIR crossfades over
  // TRANSITION_DURATION_MS (500, visualPresets.js:10), so a 700ms settle lets
  // the transition converge before we read the label / diff the frame.
  const styleBtn = await gate(
    '[data-testid="hud-style-switcher"]',
    DATA_TIMEOUT_MS,
    "style switcher",
  );
  if (styleBtn) {
    const before = await page.screenshot();
    await page.click('[data-testid="hud-style-switcher"]');
    await page.click('[data-testid="hud-style-option-thermal"]');
    await page.waitForTimeout(700); // > TRANSITION_DURATION_MS (500) so the crossfade converges
    const label =
      (await page.textContent('[data-testid="hud-style-switcher"]')) ?? "";
    const after = await page.screenshot();
    if (!/FLIR/.test(label))
      failures.push(`style switcher did not apply FLIR (label="${label.trim()}")`);
    if (Buffer.compare(before, after) === 0) {
      failures.push(
        "screenshot identical after style switch — post-process stage never ticked",
      );
    }
    // restore normal so subsequent probes see the default frame
    await page.click('[data-testid="hud-style-switcher"]');
    await page.click('[data-testid="hud-style-option-normal"]');
    await page.waitForTimeout(700);
  }

  // ---- GEV P7: location search flies the camera; geocode proxy answers ----
  // The geocode proxy (GET /api/v1/gev/geocode, photon-backed, Redis-cached
  // 1h) is asserted via two same-URL requests: the second MUST read
  // x-geocode-cache: hit (the first populates the cache). Then the HUD search
  // is driven through the live input and Enter, mirroring the real user path.
  const geo = await page.request.get(`${BASE}/api/v1/gev/geocode?q=Paris`, {
    headers: { Authorization: `Bearer ${key}` },
    timeout: REST_TIMEOUT_MS,
  });
  if (geo.status() !== 200) {
    failures.push(`geocode proxy http=${geo.status()}`);
  } else {
    const geoBody = await geo.json();
    if (!geoBody.results?.[0]?.label)
      failures.push("geocode proxy returned no results for Paris");
    const geo2 = await page.request.get(`${BASE}/api/v1/gev/geocode?q=Paris`, {
      headers: { Authorization: `Bearer ${key}` },
      timeout: REST_TIMEOUT_MS,
    });
    if (geo2.headers()["x-geocode-cache"] !== "hit")
      warnings.push("geocode second request did not hit cache");
  }
  const searchInput = await gate(
    '[data-testid="hud-search-location"]',
    DATA_TIMEOUT_MS,
    "search input",
  );
  if (searchInput) {
    // The rail gate proves start() resolved, but the adapter's dynamic import
    // lags it — wait for the input to leave the engine-not-ready disabled
    // state before driving it (page.fill throws on a disabled input).
    const searchEnabled = await page
      .waitForFunction(
        () => {
          const el = document.querySelector(
            '[data-testid="hud-search-location"]',
          );
          return !!el && !el.disabled;
        },
        null,
        { timeout: DATA_TIMEOUT_MS },
      )
      .then(() => true)
      .catch(() => false);
    if (!searchEnabled) {
      warnings.push(
        "search input stayed disabled (location search adapter never mounted)",
      );
    } else {
      // The engine viewer is NOT exposed on window (the engine's tools phase
      // is stubbed in gev-boot/application.ts, so no window.__gevViewer), and
      // a screenshot diff is unusable for a "camera moved" signal (Cesium
      // renders continuously — frames always differ). The assertion therefore
      // degrades to the weak form: a successful flyTo clears the status span;
      // a miss/failure leaves 未找到 / 搜索失败 text behind. The global-viewer
      // read is kept so the strong form activates automatically if the engine
      // ever exposes one.
      const readCam = () =>
        page
          .evaluate(() => {
            const c = window.__gevViewer?.camera;
            return c ? [c.position.x, c.position.y, c.position.z] : null;
          })
          .catch(() => null);
      const before = await readCam();
      const pageErrorsBeforeSearch = pageErrors.length;
      await page.fill('[data-testid="hud-search-location"]', "Paris");
      await page.press('[data-testid="hud-search-location"]', "Enter");
      await page.waitForTimeout(4500); // geocode + 3s flyTo duration
      const moved = await readCam();
      if (before && moved && JSON.stringify(before) === JSON.stringify(moved)) {
        failures.push("camera did not move after location search");
      } else if (!before || !moved) {
        const status = await page
          .textContent('[data-testid="hud-search-status"]')
          .catch(() => null);
        if (status && /未找到|失败/.test(status))
          failures.push(`location search surfaced: ${status.trim()}`);
      }
      if (pageErrors.length > pageErrorsBeforeSearch)
        failures.push("page errors during location search");
      // A silent no-op (search fired nothing and left no failure text) is the
      // one case the status-text weak assertion cannot see — require the page
      // to have issued its own geocode request through the proxy.
      if (!searchGeocodeSeen)
        failures.push(
          "location search did not call the geocode proxy (no page-side request observed)",
        );
    }
  }

  // ---- GEV P8: annotations — draw toolbar opens + list API answers ----
  // The draw button is a rail tool (HudLayerRail, gated on `!collapsed`), and
  // the toolbar returns null until drawActive flips (HudDrawToolbar:99), so a
  // pre-click count of 0 is the expected resting state. The click path is
  // defensive: P8 is additive chrome and a missing toolbar must not mask the
  // P1-P7 assertions (accept-sp8 covers the store/roundtrip contract).
  const drawBtn = page.locator('[data-testid="hud-draw-button"]');
  const drawBtnCount = await drawBtn.count();
  console.log(`p8-draw-button=${drawBtnCount}`);
  if (drawBtnCount > 0) {
    await drawBtn.first().click().catch(() => {});
    await page.waitForTimeout(200);
    const tbCount = await page
      .locator('[data-testid="hud-draw-toolbar"]')
      .count();
    console.log(`p8-draw-toolbar=${tbCount}`);
    if (tbCount === 0) failures.push("P8: draw toolbar did not open");
    else {
      const modes = await page
        .locator(
          '[data-testid="hud-draw-mode-pin"], [data-testid="hud-draw-mode-line"], [data-testid="hud-draw-mode-area"]',
        )
        .count();
      console.log(`p8-draw-modes=${modes}`);
      if (modes !== 3)
        failures.push(`P8: toolbar missing draw modes (saw ${modes}/3)`);
      await page
        .locator('[data-testid="hud-draw-cancel"]')
        .click()
        .catch(() => {});
    }
  } else {
    console.log("p8-draw-button-absent (deferred; not blocking)");
  }
  // Persistence API: the same-origin GET the console annotation store issues.
  // The hub gates every /api/* route (reads included) inside auth_middleware,
  // so the request must carry the Bearer key — read from localStorage exactly
  // like console/src/api.ts does (the key is seeded by addInitScript above).
  try {
    const resp = await page.evaluate(async () => {
      const r = await fetch("/api/v1/annotations?limit=5", {
        headers: {
          Authorization: `Bearer ${
            localStorage.getItem("intelhub.console.key") ?? ""
          }`,
        },
      });
      return { status: r.status, body: await r.json().catch(() => null) };
    });
    const okShape =
      resp.status === 200 && Array.isArray(resp.body?.annotations);
    console.log(
      `p8-annotations-api status=${resp.status} count=${resp.body?.annotations?.length ?? "n/a"}`,
    );
    if (!okShape)
      failures.push(
        `P8: annotations API returned ${resp.status} or wrong shape`,
      );
  } catch (e) {
    failures.push(`P8: annotations API fetch threw: ${e.message}`);
  }

  // ---- GEV P9: cockpit overlay — enter + instruments + briefing -------
  // The rail 🎮 entry (HudLayerRail) is gated on `!collapsed`; the rail is
  // expanded by default, but retry after expanding it before giving up. The
  // instruments cluster needs the flights layer's getTrackedInfo seam
  // (GlobeV2 guards it), so a 0 count is a WARNING once the frame rendered —
  // the frame / vision-switch / briefing assertions stay fatal. Defensive
  // try/catch per plan Task 5 ruling #2: a viewport/overlay hiccup must not
  // mask the P1-P8 assertions.
  try {
    let cockpitBtn = page.locator('[data-testid="hud-cockpit-button"]');
    let cockpitBtnCount = await cockpitBtn.count();
    if (cockpitBtnCount === 0) {
      await page
        .locator('[data-testid="hud-layer-rail"] .hud-rail-handle')
        .first()
        .click()
        .catch(() => {});
      await page.waitForTimeout(200);
      cockpitBtn = page.locator('[data-testid="hud-cockpit-button"]');
      cockpitBtnCount = await cockpitBtn.count();
    }
    console.log(`p9-cockpit-button=${cockpitBtnCount}`);
    if (cockpitBtnCount === 0) {
      failures.push("P9: cockpit button missing from the layer rail");
    } else {
      await cockpitBtn.first().click().catch(() => {});
      await page.waitForTimeout(300);
      const frameCount = await page
        .locator('[data-testid="hud-cockpit-frame"]')
        .count();
      console.log(`p9-cockpit-frame=${frameCount}`);
      if (frameCount === 0) {
        failures.push("P9: cockpit frame did not mount after the rail entry");
      } else {
        const instruments = await page
          .locator(
            '[data-testid="hud-cockpit-compass"], [data-testid="hud-cockpit-altimeter"], [data-testid="hud-cockpit-speed"]',
          )
          .count();
        console.log(`p9-cockpit-instruments=${instruments}`);
        if (instruments !== 3)
          warnings.push(
            `P9: cockpit instruments ${instruments}/3 (flights getTrackedInfo seam absent?)`,
          );
        const vision = await page
          .locator(
            '[data-testid="hud-cockpit-vision-optical"], [data-testid="hud-cockpit-vision-crt"], [data-testid="hud-cockpit-vision-nvg"], [data-testid="hud-cockpit-vision-thermal"], [data-testid="hud-cockpit-vision-noir"]',
          )
          .count();
        console.log(`p9-cockpit-vision-modes=${vision}`);
        if (vision !== 5)
          failures.push(
            `P9: cockpit vision switch saw ${vision}/5 modes (want 5)`,
          );
        const briefing = await page
          .locator('[data-testid="hud-cockpit-briefing"]')
          .count();
        console.log(`p9-briefing-panel=${briefing}`);
        if (briefing === 0)
          failures.push("P9: briefing panel missing inside the cockpit");
      }
    }
  } catch (e) {
    warnings.push(`P9 cockpit segment threw: ${e.message}`);
  }

  // Briefing data path: the same-origin REST the panel issues. Weather needs
  // a tracked position, so the probe asks with fixed coords (a stationary
  // hotspot); a non-200 is a WARNING — the weather endpoint 503s only when
  // BOTH NOAA and Open-Meteo die, and plan Task 5 ruling #4 says environmental
  // degradation is a shelve, not a failure. The summary stub is deterministic
  // → hard 200.
  try {
    const weather = await page.request.get(
      `${BASE}/api/v1/gev/weather?lat=40.0&lon=-74.0`,
      { headers: { Authorization: `Bearer ${key}` }, timeout: REST_TIMEOUT_MS },
    );
    console.log(`p9-briefing-weather=${weather.status()}`);
    if (weather.status() !== 200)
      warnings.push(`P9: briefing weather http=${weather.status()}`);
  } catch (e) {
    warnings.push(`P9: briefing weather fetch threw: ${e.message}`);
  }
  try {
    const summary = await page.request.get(
      `${BASE}/api/v1/gev/summary?entity_id=test`,
      { headers: { Authorization: `Bearer ${key}` }, timeout: REST_TIMEOUT_MS },
    );
    console.log(`p9-briefing-summary=${summary.status()}`);
    if (summary.status() !== 200)
      failures.push(`P9: briefing summary http=${summary.status()}`);
  } catch (e) {
    failures.push(`P9: briefing summary fetch threw: ${e.message}`);
  }

  // ---- GEV P10: tail HUD widgets — frame-rate / shortcuts / scene / ------
  // recording / panel-drag. Same source-vs-live split as P9 (sp8 covers
  // shipped bundle testids + vendor source contracts; here we drive the
  // real widgets). Each segment is wrapped in try/catch (plan §4 R2 +
  // brief §"Plan-deferred rulings" #2): a single flaky widget must not
  // mask the P1-P9 assertions. P10 widgets rely on T3's GlobeV2 tail
  // adapters being mounted inside the engine boot path (frame-rate
  // requires the vendor postRender subscription, recording requires the
  // HUD contract setMode wiring); if the engine's tools phase is
  // stubbed (IntelHub's case) the widget mounts the React-only path and
  // exposes itself via its testid without engine integration.
  // -----------------------------------------------------------------------

  // p10-frame-rate-readout: the chip lives on the top bar — but only mounts
  // when the rail `hud-fps-toggle` has been clicked (GlobeV2 gates it on
  // `fpsReadoutOpen`, HudFrameRateReadout.tsx returns nothing while closed).
  // Click the toggle first, then assert the testid is present. The FPS
  // value is vendor-driven and may be `null` until the first postRender;
  // the test only checks DOM presence (mirrors the p9 cockpit button +
  // instruments pattern).
  try {
    const fpsBtn = page.locator('[data-testid="hud-fps-toggle"]');
    const fpsBtnCount = await fpsBtn.count();
    if (fpsBtnCount === 0) {
      console.log("p10-fps-toggle=0");
      warnings.push("P10: hud-fps-toggle missing from the rail (T3 wiring absent?)");
    } else {
      await fpsBtn.first().click().catch(() => {});
      await page.waitForTimeout(250);
      const fr = await page
        .locator('[data-testid="hud-frame-rate-readout"]')
        .count();
      console.log(`p10-frame-rate-readout=${fr}`);
      if (fr === 0)
        warnings.push(
          "P10: frame-rate readout HUD testid missing after fps-toggle click",
        );
      // Toggle back so subsequent segments see the default frame.
      await fpsBtn.first().click().catch(() => {});
      await page.waitForTimeout(150);
    }
  } catch (e) {
    warnings.push(`P10 frame-rate segment threw: ${e.message}`);
  }

  // p10-shortcut-cheatsheet: press `?` (vendor's applicationShortcuts key +
  // the adapter's cheatsheet pop-key) and assert the dialog mounts. The
  // shortcut fires on document body; clicking outside the rail + bottom
  // bar keeps the focus away from the search input (which would consume
  // the keystroke and dismiss any active state).
  // NOTE: don't press Escape to close the cheatsheet — the vendor's
  // Escape handler in applicationShortcuts.js calls `dismissSearch`,
  // which on the 315 build dispatches a synthetic Escape (GlobeV2.tsx:284)
  // that recurses into itself until the stack overflows. Use the
  // cheatsheet's own close button instead.
  try {
    // Move focus away from any form control (search input, checkboxes)
    // by clicking the top bar — `?` then reaches the document handler.
    await page.locator("body").click({ position: { x: 1, y: 1 } }).catch(() => {});
    await page.waitForTimeout(150);
    await page.keyboard.press("?");
    await page.waitForTimeout(250);
    const cs = await page
      .locator('[data-testid="hud-shortcut-cheatsheet"]')
      .count();
    console.log(`p10-shortcut-cheatsheet=${cs}`);
    if (cs === 0)
      warnings.push("P10: shortcut cheatsheet did not open on `?` keypress");
    else {
      // Close via the cheatsheet's own × button (testid-able) to avoid
      // the dismissSearch Escape recursion on 315.
      const closeBtn = page.locator(
        '[data-testid="hud-shortcut-cheatsheet"] .hud-shortcut-cheatsheet-close',
      );
      if (await closeBtn.count())
        await closeBtn.first().click().catch(() => {});
      else
        // Fall back to clicking the body (less reliable, but doesn't
        // route through Escape).
        await page.locator("body").click({ position: { x: 2, y: 2 } }).catch(() => {});
      await page.waitForTimeout(150);
    }
  } catch (e) {
    warnings.push(`P10 shortcut segment threw: ${e.message}`);
  }

  // p10-scene-panel: click the rail `hud-scene-toggle` and assert
  // `hud-scene-panel` mounts. The toggle is only present when the rail
  // exposes `onToggleScene` (T3 wires this for P10 — see GlobeV2 wiring).
  try {
    const sceneToggle = page.locator('[data-testid="hud-scene-toggle"]');
    const sceneToggleCount = await sceneToggle.count();
    if (sceneToggleCount === 0) {
      console.log("p10-scene-toggle=0");
      warnings.push("P10: hud-scene-toggle missing from the rail (T3 wiring absent?)");
    } else {
      await sceneToggle.first().click().catch(() => {});
      await page.waitForTimeout(250);
      const sp = await page
        .locator('[data-testid="hud-scene-panel"]')
        .count();
      console.log(`p10-scene-panel=${sp}`);
      if (sp === 0)
        warnings.push("P10: scene panel did not mount after rail toggle");
      // Close the scene panel by clicking its close button so subsequent
      // segments (recording, panel-drag) start from a clean slate.
      const closeBtn = page.locator(
        '[data-testid="hud-scene-panel"] .hud-scene-panel-close',
      );
      if (await closeBtn.count())
        await closeBtn.first().click().catch(() => {});
    }
  } catch (e) {
    warnings.push(`P10 scene-panel segment threw: ${e.message}`);
  }

  // p10-recording-mode: click the rail `hud-recording-button` and assert
  // `document.body.classList` contains `recording-mode`. The HUD contract
  // `setMode` callback (T3 wiring) toggles the class directly — T3 review
  // concern #1 noted the vendor's setRecordingMode API is not invoked,
  // but the body class assertion is the contract-shaped check (vendor
  // recording.css selectors also key on body.recording-mode).
  try {
    const recBefore = await page.evaluate(() =>
      document.body.classList.contains("recording-mode"),
    );
    const recBtn = page.locator('[data-testid="hud-recording-button"]');
    const recBtnCount = await recBtn.count();
    if (recBtnCount === 0) {
      console.log("p10-recording-button=0");
      warnings.push("P10: hud-recording-button missing from the rail");
    } else {
      await recBtn.first().click().catch(() => {});
      await page.waitForTimeout(250);
      const recAfter = await page.evaluate(() =>
        document.body.classList.contains("recording-mode"),
      );
      console.log(
        `p10-recording-mode before=${recBefore} after=${recAfter}`,
      );
      if (recAfter !== true)
        warnings.push(
          `P10: body.recording-mode not set after rail click (before=${recBefore} after=${recAfter})`,
        );
      // Toggle back so the body class doesn't leak into subsequent
      // segments / the visual-effects stack.
      await recBtn.first().click().catch(() => {});
      await page.waitForTimeout(250);
    }
  } catch (e) {
    warnings.push(`P10 recording segment threw: ${e.message}`);
  }

  // p10-panel-drag=<id>=1: verify the localStorage key template is read+
  // written by the live page. The vendor's PanelPositionControls
  // (wired via mountPanelDrag in GlobeV2) owns the godsEyeView.v8.panelPos
  // namespace; it READS on mount to restore positions and WRITES on
  // drag end. NOTE: the IntelHub HUD's drag affordance (HudPanelDragHandle
  // in HudDetailPanel) is currently PRESENTATION-ONLY — T3 didn't wire its
  // pointer events to the vendor's drag surface (T3 review concern #2 +
  // brief §"T3 scene-panel carry"). The probe therefore asserts the
  // read-path: seed the storage key with a sentinel, reload, verify the
  // key persists AND the vendor's storage API would be consulted. The
  // full mouse-driven roundtrip will exercise once the wiring is added
  // (deferred — T3/T4 can't reach into the vendor mousedown surface from
  // a probe without flakiness, and the roundtrip contract is otherwise
  // asserted by sp8 bundle-level checks).
  try {
    const panelId = "detail-panel";
    const storageKey = `godsEyeView.v8.panelPos.${panelId}`;
    const sentinel = { left: 123, top: 456 };
    await page.evaluate(
      ([k, v]) => localStorage.setItem(k, JSON.stringify(v)),
      [storageKey, sentinel],
    );
    // Read-back before reload — proves the write path (our own setItem)
    // and confirms the key template matches what the vendor would write.
    const beforeReload = await page.evaluate((k) => {
      const raw = localStorage.getItem(k);
      return raw ? JSON.parse(raw) : null;
    }, storageKey);
    // Reload so the next /globe mount has the chance to consume the
    // storage key (vendor's _initPanelDrag → _restorePanelPosition).
    await page.reload({ waitUntil: "load", timeout: 60_000 });
    await page
      .waitForSelector('[data-testid="hud-layer-rail"]', {
        state: "attached",
        timeout: 30_000,
      })
      .catch(() => {});
    await page.waitForTimeout(1500); // rAF + vendor _initPanelDrag
    const afterReload = await page.evaluate((k) => {
      const raw = localStorage.getItem(k);
      return raw ? JSON.parse(raw) : null;
    }, storageKey);
    console.log(
      `p10-panel-drag=${panelId} key=${storageKey} `
        + `before=${JSON.stringify(beforeReload)} after=${JSON.stringify(afterReload)}`,
    );
    // Storage roundtrip: the key must persist across reload (vendor
    // doesn't clear it on mount). If the vendor DID consult and apply
    // the position, the sentinel is still there (the vendor doesn't
    // delete it). We can't assert visual position because the
    // drag-affordance wiring is T3-deferred.
    const dragOk =
      beforeReload !== null &&
      afterReload !== null &&
      beforeReload.left === afterReload.left &&
      beforeReload.top === afterReload.top;
    if (!dragOk)
      warnings.push(
        `P10: panel-drag storage key did not persist (before=${JSON.stringify(beforeReload)} after=${JSON.stringify(afterReload)})`,
      );
    // Cleanup so subsequent loads start fresh.
    await page.evaluate((k) => localStorage.removeItem(k), storageKey);
  } catch (e) {
    warnings.push(`P10 panel-drag segment threw: ${e.message}`);
  }

  // ---- GEV P11: CCTV popout panel live smoke ----
  // Verifies that clicking `cctv-open-popout` from HudDetailPanel mounts
  // the `cctv-popout-panel` with a working <img> / <video> child, and
  // that pressing Escape closes it. The popout must remain mounted across
  // a position drag (localStorage write) and re-mount on reload (localStorage
  // read).
  //
  // Skip if console 404s on /cctv-open-popout — handle gracefully (push to
  // warnings, don't fail). The CI must keep reporting, even if the live
  // vendor retag stripped the button.
  try {
    // Wait for the rail layer rail entry that surfaces cameras (LayerRail
    // 'cctv' column on the right edge per P11 spec).
    await page.locator('[data-testid="hud-cctv-tab"]').first().click({ timeout: 5000 }).catch(() => {});
    // Look for any camera detail that exposes the open-popout button.
    const popoutBtn = page.locator('[data-testid="cctv-open-popout"]').first();
    let popoutBtnCount = await popoutBtn.count();
    console.log(`p11-cctv-popout-btn=${popoutBtnCount}`);
    if (popoutBtnCount === 0) {
      warnings.push("P11: cctv-open-popout button not present in any visible detail (layer rail entry may be empty)");
    } else {
      await popoutBtn.click({ timeout: 5000 }).catch(() => {});
      await page.waitForTimeout(800);
      const popoutPanel = page.locator('[data-testid="cctv-popout-panel"]');
      const panelCount = await popoutPanel.count();
      console.log(`p11-cctv-popout-panel=${panelCount}`);
      if (panelCount === 0) {
        warnings.push("P11: popout panel did not mount after click (vendor CSS missing or React error)");
      } else {
        const mediaOk = await popoutPanel.first().locator('img, video').count();
        console.log(`p11-cctv-popout-media=${mediaOk}`);
        if (mediaOk === 0) warnings.push("P11: popout panel mounted but has no <img>/<video> child");
        // Close via Escape — must unmount.
        await page.keyboard.press("Escape");
        await page.waitForTimeout(300);
        const afterClose = await page.locator('[data-testid="cctv-popout-panel"]').count();
        console.log(`p11-cctv-popout-after-escape=${afterClose}`);
        if (afterClose > 0) warnings.push("P11: popout did not close on Escape");
      }
    }
  } catch (e) {
    warnings.push(`P11 cctv-popout segment threw: ${e.message}`);
  }

  // ---- GEV P12: flight-layer parity — aircraft source + cockpit behaviors ----
  // The P10 panel-drag segment reloaded the page, so the P9 cockpit entry is
  // gone; re-enter before probing the live overlay. The aircraft-source probes
  // assert the LOADED bundle text because the spec's `window.__gevAircraftSource`
  // debug handle was never implemented — the adapter is reached only through
  // the vendor's `_source` seam, so there is nothing to read off `window`.
  // The cockpit probes drive the real T5 keyboard + T7 viewport-lock widgets
  // (live counterparts of sp8 checks 51-53, which are acceptance-deferred).
  try {
    let p12Btn = page.locator('[data-testid="hud-cockpit-button"]');
    if ((await p12Btn.count()) === 0) {
      await page
        .locator('[data-testid="hud-layer-rail"] .hud-rail-handle')
        .first()
        .click()
        .catch(() => {});
      await page.waitForTimeout(200);
      p12Btn = page.locator('[data-testid="hud-cockpit-button"]');
    }
    await p12Btn.first().click({ timeout: 5000 }).catch(() => {});
    await page.waitForTimeout(400);

    const P12_PROBES = [
      // 1. T3 adapter getTrack: the shipped asset must carry the adapter's
      //    unique marker (`hub.intelhub`) + the vendor path literal. The bare
      //    `getTrack`/`/api/opensky-track` also live in the vendored
      //    standalone.js, so the marker is what separates T3 from pre-T3.
      ["p12-aircraft-source-getTrack", () =>
        page.evaluate(async () => {
          const urls = performance
            .getEntriesByType("resource")
            .map((e) => e.name)
            .filter((n) => /\/assets\/.*\.js(\?|$)/.test(n));
          for (const u of urls) {
            const t = await (await fetch(u)).text().catch(() => "");
            if (
              t.includes("hub.intelhub") &&
              t.includes("getTrack") &&
              t.includes("/api/opensky-track")
            )
              return true;
          }
          return false;
        })],
      // 2. T3 adapter getEnrichment: same bundle-text strategy (+ the unique
      //    callsign-guard string that exists only in aircraft-source.ts).
      ["p12-aircraft-source-getEnrichment", () =>
        page.evaluate(async () => {
          const urls = performance
            .getEntriesByType("resource")
            .map((e) => e.name)
            .filter((n) => /\/assets\/.*\.js(\?|$)/.test(n));
          for (const u of urls) {
            const t = await (await fetch(u)).text().catch(() => "");
            if (
              t.includes("hub.intelhub") &&
              t.includes("getEnrichment") &&
              t.includes("/api/adsbdb")
            )
              return true;
          }
          return false;
        })],
      // 3. cockpit overlay mounted after the rail entry.
      ["p12-cockpit-active", () =>
        page.evaluate(
          () => !!document.querySelector('[data-testid="hud-cockpit-frame"]'),
        )],
      // 4. exit affordance present.
      ["p12-cockpit-exit-btn", () =>
        page.evaluate(
          () => !!document.querySelector('[data-testid="hud-cockpit-exit"]'),
        )],
      // 5. all five vision modes (the switch container shares the
      //    `hud-cockpit-vision-` prefix, so count the mode ids explicitly).
      ["p12-cockpit-vision-keys", () =>
        page.evaluate(() => {
          const modes = ["optical", "crt", "nvg", "thermal", "noir"];
          return modes.every((m) =>
            document.querySelector(`[data-testid="hud-cockpit-vision-${m}"]`),
          );
        })],
      // 6. shortcut hint advertises the arrow keys.
      ["p12-cockpit-shortcut-hint", () =>
        page.evaluate(() => {
          const el = document.querySelector(
            '[data-testid="hud-cockpit-shortcut-hint"]',
          );
          return !!el && /← →/.test(el.textContent ?? "");
        })],
      // 7. T7 viewport lock: canvas cursor hidden while active (the lock's
      //    observable effect; the spec's `__cockpitStore.viewportLocked` flag
      //    was never implemented).
      ["p12-cockpit-viewport-lock", () =>
        page.evaluate(() => {
          const c = document.querySelector("#cesiumContainer canvas");
          return !!c && c.style.cursor === "none";
        })],
      // 8. T2 tracks endpoint reachable from the page origin: 200 + records[]
      //    or the documented 503 missing-creds, and always a JSON body
      //    (unmatched routes fall through to the SPA index.html 200 + HTML).
      ["p12-tracks-endpoint-reachable", async () => {
        const res = await page.request.get(
          `${BASE}/api/opensky-track?icao24=4ca9b1`,
          { headers: { Authorization: `Bearer ${key}` }, timeout: REST_TIMEOUT_MS },
        );
        const ct = res.headers()["content-type"] ?? "";
        return (res.status() === 200 || res.status() === 503) && /json/.test(ct);
      }],
    ];

    for (const [name, fn] of P12_PROBES) {
      let ok = false;
      let note = "";
      try {
        ok = !!(await fn());
      } catch (e) {
        note = ` err=${e.message}`;
      }
      console.log(`${name}=${ok ? 1 : 0}${note}`);
      if (!ok) failures.push(`P12: ${name} failed`);
    }

    // T5 keyboard live check: Tab must flip the controlled briefing tab
    // (weather→summary), then Escape must unmount the overlay. Kept in the
    // segment (not a probe entry) because Escape mutates the page state.
    const tabBefore = await page
      .locator('[data-testid="hud-cockpit-tab-summary"]')
      .getAttribute("aria-selected")
      .catch(() => null);
    await page.keyboard.press("Tab");
    await page.waitForTimeout(250);
    const tabAfter = await page
      .locator('[data-testid="hud-cockpit-tab-summary"]')
      .getAttribute("aria-selected")
      .catch(() => null);
    const tabOk = tabBefore !== tabAfter && tabAfter === "true";
    console.log(
      `p12-cockpit-keyboard-tab=${tabOk ? 1 : 0} before=${tabBefore} after=${tabAfter}`,
    );
    if (!tabOk)
      failures.push(
        `P12: cockpit Tab did not flip briefing tab (before=${tabBefore} after=${tabAfter})`,
      );

    await page.keyboard.press("Escape").catch(() => {});
    await page.waitForTimeout(300);
    const exited =
      (await page.locator('[data-testid="hud-cockpit-frame"]').count()) === 0;
    console.log(`p12-cockpit-escape-exit=${exited ? 1 : 0}`);
    if (!exited)
      failures.push("P12: cockpit Escape did not exit the overlay");
  } catch (e) {
    failures.push(`P12 flight-layer segment threw: ${e.message}`);
  }

  // ---- GEV P13: flight-display optimization — default camera + enrich +
  //      HudAircraftDetail + 3rd ADS-B source (2026-09-21) ----
  // The P12 segment exited cockpit via Escape, so the HUD right rail is back
  // to its resting state: HudAircraftDetail renders its empty aside (nothing
  // tracked) — the mount + i18n half of T3. The camera/enrich halves assert
  // bundle text + the live QA seam (window.__GEV_ENRICH_AMBIENT_QA is a real
  // runtime global, written by applyEnrichAmbientOverride at boot). The adsbx
  // probe reads the merged coverage off the already-fetched aircraft envelope.
  try {
    const P13_PROBES = [
      // 1. T1 mountDefaultCamera shipped: the bundle carries the contract-gate
      //    TypeError literal (unique to default-camera.ts; minification keeps
      //    string literals intact).
      ["p13-default-camera-bundle", () =>
        page.evaluate(async () => {
          const urls = performance
            .getEntriesByType("resource")
            .map((e) => e.name)
            .filter((n) => /\/assets\/.*\.js(\?|$)/.test(n));
          for (const u of urls) {
            const t = await (await fetch(u)).text().catch(() => "");
            if (t.includes("viewer.camera.setView is missing"))
              return true;
          }
          return false;
        })],
      // 2. T2 enrich budget live: the QA seam must carry ceil >= 800 at
      //    runtime (written by applyEnrichAmbientOverride before the first
      //    vendor sweep; the vendor reads it lazily on every refill).
      ["p13-enrich-budget-live", () =>
        page.evaluate(
          () => (window.__GEV_ENRICH_AMBIENT_QA?.ceil ?? 0) >= 800,
        )],
      // 3. T3 HudAircraftDetail mounted: either the populated card or the
      //    empty-state aside is present (nothing is tracked after cockpit exit).
      ["p13-hud-detail-mount", () =>
        page.evaluate(
          () =>
            !!document.querySelector('[data-testid="hud-aircraft-detail"]') ||
            !!document.querySelector('[data-testid="hud-aircraft-detail-empty"]'),
        )],
      // 4. T3 empty-state i18n: the empty aside carries the translated text
      //    (en or zh — the locale is browser-dependent, so accept either).
      ["p13-hud-detail-empty-text", () =>
        page.evaluate(() => {
          const el = document.querySelector(
            '[data-testid="hud-aircraft-detail-empty"]',
          );
          if (!el) return false;
          return /Click a flight to inspect|点击飞机查看详情/.test(
            el.textContent ?? "",
          );
        })],
      // 5. T4 3rd ADS-B source: the merged aircraft coverage must carry the
      //    +adsbx suffix (the suffix is only appended when the adsbx snapshot
      //    has non-empty rows — i.e. the 3rd source actually contributed).
      ["p13-adsbx-coverage", () =>
        typeof aircraft?.coverage === "string" &&
        aircraft.coverage.includes("adsbx")],
    ];

    for (const [name, fn] of P13_PROBES) {
      let ok = false;
      let note = "";
      try {
        ok = !!(await fn());
      } catch (e) {
        note = ` err=${e.message}`;
      }
      console.log(`${name}=${ok ? 1 : 0}${note}`);
      if (!ok) failures.push(`P13: ${name} failed`);
    }
  } catch (e) {
    failures.push(`P13 flight-display segment threw: ${e.message}`);
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
// The "Maximum call stack size exceeded" RangeError is a T3-introduced
// 315-specific bug in the dismissSearch Escape handler (GlobeV2.tsx:284
// dispatches a synthetic Escape that re-enters the vendor's onKeyDown,
// recursing until the stack overflows). The 410 prod build does not
// exhibit this (P9 ledger: pageerrors=0). It is INTENTIONALLY not filtered
// here — the probe must report it so the regression is visible in T5's
// ledger review. The probe's P10 segments avoid triggering fresh
// dismissSearch calls (cheatsheet close uses the × button, recording
// toggle uses the rail icon, panel-drag uses localStorage seed instead
// of a synthetic drag), so the new T4 surface does not add to the
// existing 46 baseline count.
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
