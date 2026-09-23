// probe-layer.mjs — generic Cesium globe layer motion probe via CDP screenshot diff.
//
// Generalized from probe-motion.mjs. Each layer has its own threshold + an
// optional pre-flight data check (asserts the layer's hub REST endpoint
// returns ≥1 record before we trust a zero-diff screenshot).
//
// CDP `Page.captureScreenshot` bypasses playwright's stability wait that
// stalls on Cesium's continuous rAF (holdContinuousRender).
//
// Usage:
//   node probe-layer.mjs --url=URL --layer=NAME --out=DIR
// Env:
//   PROBE_API_KEY        ihk_<hex> (passed through console SPA auth gate)
//   PROBE_WAIT_INITIAL   ms to settle after page load (default 30000)
//   PROBE_WAIT_BETWEEN   ms between t0 and t1 (default 10000)
//   PROBE_LAYER_CONFIG   JSON path overriding LAYER_PROBES table (test only)
//
// Exit codes:
//   0  PASS (pct >= threshold AND preflight ok)
//   1  FAIL (pct below threshold OR byte-identical)
//   2  PREFLIGHT_FAIL (data endpoint missing/empty)
//   3  PROBE_ERROR (chromium/timeout/runtime)
import fs from "node:fs";
import path from "node:path";
import { chromium } from "playwright";
import { PNG } from "pngjs";

const URL = (() => {
  const a = process.argv.find((x) => x.startsWith("--url="));
  return a ? a.slice("--url=".length) : (process.env.PROBE_URL ?? "http://10.10.10.35:8800/globe");
})();
const LAYER = (() => {
  const a = process.argv.find((x) => x.startsWith("--layer="));
  return a ? a.slice("--layer=".length) : "aircraft";
})();
const OUT_DIR = (() => {
  const a = process.argv.find((x) => x.startsWith("--out="));
  return a ? a.slice("--out=".length) : (process.env.PROBE_OUT ?? "/tmp/probe-layer");
})();
const WAIT_MS_INITIAL = parseInt(process.env.PROBE_WAIT_INITIAL ?? "30000", 10);
const WAIT_MS_BETWEEN = parseInt(process.env.PROBE_WAIT_BETWEEN ?? "10000", 10);
const API_KEY = process.env.PROBE_API_KEY ?? "";

// Per-layer config. threshold_pct = minimum pixel-channel-delta-gt-5 fraction
// over the canvas to call it "moving". preflight (optional) = REST call that
// must return ≥1 row before the probe trusts motion; zero-data layers produce
// zero-diff screenshots that would otherwise pass falsely.
const LAYER_PROBES = {
  aircraft: {
    desc: "aircraft dead-reckoning interpolation (flights layer)",
    threshold_pct: 0.3,
    preflight: { path: "/api/v1/globe/aircraft", min_rows: 1 },
  },
  satellites: {
    desc: "TLE-driven orbit propagation (satellites layer)",
    threshold_pct: 0.3,
    preflight: { path: "/api/v1/gev/celestrak/stations", min_rows: 1 },
  },
  vessels: {
    desc: "AIS live tracking interpolation (ais-live-vessels layer)",
    threshold_pct: 0.3,
    preflight: { path: "/api/v1/gev/ais-live", min_rows: 1 },
  },
  traffic: {
    desc: "flow heatmap poll refresh (traffic layer)",
    threshold_pct: 0.5, // flow tiles update slower than per-frame motion
    preflight: { path: null, min_rows: 0 }, // traffic endpoint path TBD in P22
  },
};

const cfg = LAYER_PROBES[LAYER];
if (!cfg) {
  console.error(`[probe-layer] unknown layer: ${LAYER}; valid: ${Object.keys(LAYER_PROBES).join(", ")}`);
  process.exit(3);
}

fs.mkdirSync(OUT_DIR, { recursive: true });

console.log(`[probe-layer] url=${URL} layer=${LAYER} out=${OUT_DIR}`);
console.log(`[probe-layer] desc: ${cfg.desc}`);
console.log(`[probe-layer] threshold: ${cfg.threshold_pct}%  waits: initial=${WAIT_MS_INITIAL}ms between=${WAIT_MS_BETWEEN}ms`);

// Preflight: confirm the layer's data source has rows. If empty, mark the
// layer as PREFLIGHT_FAIL — motion probe would pass vacuously (zero diff
// because nothing renders) and silently mask a dead data source.
// Some endpoints (celestrak /gev/celestrak/{group}) return TLE plain text
// rather than JSON; handle both via Content-Type sniffing.
if (cfg.preflight?.path && API_KEY) {
  try {
    const r = await fetch(`${URL.replace(/\/globe$/, "")}${cfg.preflight.path}`, {
      headers: { Authorization: `Bearer ${API_KEY}` },
      signal: AbortSignal.timeout(5000),
    });
    if (!r.ok) {
      console.log(`[probe-layer] PREFLIGHT_FAIL: ${cfg.preflight.path} status=${r.status}`);
      process.exit(2);
    }
    const ct = r.headers.get("content-type") ?? "";
    const text = await r.text();
    let rows = 0;
    if (ct.includes("application/json")) {
      const body = JSON.parse(text);
      rows = Array.isArray(body) ? body.length
        : Array.isArray(body?.aircraft) ? body.aircraft.length
        : Array.isArray(body?.vessels) ? body.vessels.length
        : Array.isArray(body?.stations) ? body.stations.length
        : Array.isArray(body?.data) ? body.data.length
        : Array.isArray(body?.rows) ? body.rows.length  // AIS live / globe aircraft convention
        : 0;
    } else if (ct.includes("text/plain")) {
      // TLE format: 3 lines per satellite, line 2 starts with "1 ", line 3
      // with "2 ". Count by either prefix for resilience against leading
      // blank lines / BOM / etc.
      rows = text.split("\n").filter((l) => /^1\s/.test(l)).length;
    } else {
      // Unknown shape: any non-empty body counts as 1+ row.
      rows = text.trim().length > 0 ? 1 : 0;
    }
    if (rows < cfg.preflight.min_rows) {
      console.log(`[probe-layer] PREFLIGHT_FAIL: ${cfg.preflight.path} rows=${rows} < ${cfg.preflight.min_rows} ct=${ct}`);
      process.exit(2);
    }
    console.log(`[probe-layer] preflight ok: ${cfg.preflight.path} rows=${rows} ct=${ct.split(";")[0]}`);
  } catch (e) {
    console.log(`[probe-layer] PREFLIGHT_FAIL: ${cfg.preflight.path} error=${e.message.split("\n")[0]}`);
    process.exit(2);
  }
}

const consoleErrors = [];
const browser = await chromium.launch({
  headless: true,
  args: [
    "--no-sandbox",
    "--disable-dev-shm-usage",
    // Cesium requires WebGL; headless chromium needs SwiftShader as the GL
    // backend. The `--use-gl=angle --use-angle=swiftshader` pair is the
    // ANGLE-SwiftShader bridge Cesium's WebGL2 init looks for; the bare
    // `--use-gl=swiftshader` flag fails on the same VM (CesiumWidget init
    // throws "The browser supports WebGL, but initialization failed"). The
    // `--enable-unsafe-swiftshader` flag is required for SwiftShader to be
    // considered available in newer Chromium. probe-gev.mjs:1484 documents
    // the same SwiftShader-missing warning for the GEV reference project.
    "--use-gl=angle",
    "--use-angle=swiftshader",
    "--enable-features=Vulkan",
    "--enable-unsafe-swiftshader",
  ],
});
console.log(`[probe-layer] chromium launched`);
try {
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => consoleErrors.push(`pageerror: ${e.message}`));
  page.on("console", (m) => {
    if (m.type() === "error") consoleErrors.push(`console.error: ${m.text()}`);
  });

  console.log(`[probe-layer] loading ${URL}`);
  await page.goto(URL, { waitUntil: "networkidle", timeout: 120_000 });

  // Auth gate: console SPA prompts for ihk_ key on first visit.
  if (API_KEY) {
    try {
      const input = await page.waitForSelector(
        'input[placeholder^="ihk_"], input[type="password"]',
        { timeout: 5_000 },
      );
      if (input) {
        await input.fill(API_KEY);
        console.log(`[probe-layer] filled API key`);
        const connectBtn = await page.$('button:has-text("Connect")')
          ?? await page.$('button[type="submit"]');
        if (connectBtn) {
          await connectBtn.click();
          console.log(`[probe-layer] clicked Connect`);
          await page.waitForLoadState("networkidle", { timeout: 30_000 });
          await page.waitForTimeout(2_000);
        }
      }
    } catch (e) {
      console.log(`[probe-layer] no auth gate visible (${e.message.split("\n")[0]})`);
    }
  }

  const diag = await page.evaluate(() => {
    const root = document.getElementById("root");
    const canvas = document.querySelector("canvas");
    return {
      rootInnerLen: root ? root.innerHTML.length : -1,
      canvasFound: !!canvas,
      canvasW: canvas ? canvas.width : 0,
      canvasH: canvas ? canvas.height : 0,
      webgl: (() => {
        const c = document.createElement("canvas");
        return !!(c.getContext("webgl2") || c.getContext("webgl"));
      })(),
    };
  }).catch((e) => ({ error: String(e) }));
  console.log(`[probe-layer] diag: ${JSON.stringify(diag)}`);
  if (consoleErrors.length) {
    console.log(`[probe-layer] console_errors so far (${consoleErrors.length}):`);
    for (const e of consoleErrors.slice(0, 4)) console.log(`  ${e}`);
  }

  try {
    await page.waitForSelector("canvas", { timeout: 60_000 });
  } catch (err) {
    await page.screenshot({ path: path.join(OUT_DIR, `${LAYER}-diag-no-canvas.png`), timeout: 10_000 }).catch(() => {});
    console.log(`[probe-layer] canvas never appeared; diag saved`);
    throw err;
  }
  console.log(`[probe-layer] canvas found; waiting ${WAIT_MS_INITIAL / 1000}s for first poll cycle`);
  await page.waitForTimeout(WAIT_MS_INITIAL);

  // CDP capture bypasses playwright stability wait (Cesium rAF keeps canvas
  // un-stable forever — see AGENTS.md "Leaflet 视图未就绪禁动视图" lesson).
  async function snap(file) {
    const session = await page.context().newCDPSession(page);
    const result = await session.send("Page.captureScreenshot", {
      format: "png",
      captureBeyondViewport: false,
    });
    await session.detach().catch(() => {});
    fs.writeFileSync(file, Buffer.from(result.data, "base64"));
  }

  const t0Path = path.join(OUT_DIR, `${LAYER}-t0.png`);
  await snap(t0Path);
  console.log(`[probe-layer] t0: ${t0Path}`);

  await page.waitForTimeout(WAIT_MS_BETWEEN);

  const t1Path = path.join(OUT_DIR, `${LAYER}-t1.png`);
  await snap(t1Path);
  console.log(`[probe-layer] t1: ${t1Path}`);

  // Pixel diff: count pixels where any RGB channel changes by > 5.
  const a = PNG.sync.read(fs.readFileSync(t0Path));
  const b = PNG.sync.read(fs.readFileSync(t1Path));
  if (a.width !== b.width || a.height !== b.height) {
    console.log(`[probe-layer] FAIL: viewport size differs ${a.width}x${a.height} vs ${b.width}x${b.height}`);
    process.exit(1);
  }
  const total = a.width * a.height;
  let changed = 0;
  for (let i = 0; i < a.data.length; i += 4) {
    if (
      Math.abs(a.data[i] - b.data[i]) > 5 ||
      Math.abs(a.data[i + 1] - b.data[i + 1]) > 5 ||
      Math.abs(a.data[i + 2] - b.data[i + 2]) > 5
    ) {
      changed++;
    }
  }
  const pct = (changed / total) * 100;
  const byteIdentical = fs.readFileSync(t0Path).equals(fs.readFileSync(t1Path));
  console.log(`\n=== probe-layer result (${LAYER}) ===`);
  console.log(`byte_identical: ${byteIdentical}`);
  console.log(`viewport: ${a.width}x${a.height}`);
  console.log(`pixels_with_chan_delta_gt_5: ${changed} / ${total} (${pct.toFixed(3)}%)`);
  console.log(`threshold: ${cfg.threshold_pct}%`);
  console.log(`console_errors: ${consoleErrors.length}`);
  if (consoleErrors.length) for (const e of consoleErrors.slice(0, 4)) console.log(`  ${e}`);

  if (byteIdentical || pct < cfg.threshold_pct) {
    console.log(`\nMOTION FAIL — ${pct.toFixed(3)}% < ${cfg.threshold_pct}% threshold`);
    process.exit(1);
  }
  console.log(`\nMOTION VERIFIED (${LAYER}) — ${pct.toFixed(3)}% pixels changed across ${WAIT_MS_BETWEEN / 1000}s window`);
  process.exit(0);
} catch (e) {
  console.log(`[probe-layer] PROBE_ERROR: ${e.message.split("\n")[0]}`);
  process.exit(3);
} finally {
  await browser.close().catch(() => {});
}
