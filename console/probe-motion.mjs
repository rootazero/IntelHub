#!/usr/bin/env node
// probe-motion.mjs — GEV P21 T1 hard-prove: aircraft on the globe page move.
//
// Pre-fix (production incident 2026-09-23): `_positionHistory` never advanced
// past the first 1970-epoch fix because the merged /globe/aircraft envelope
// had no `ts` field. Per-frame `_deadReckon` computed the same Cartesian3
// every frame (dtSec clamped to 60 s by `staleCoastLimitSeconds`), the
// `bb.position` 1 m gate never fired, aircraft froze at their initial poll
// position.
//
// Post-fix: envelope carries a fresh RFC3339 `ts`; the adapter derives
// per-record `positionTimeMs`; two consecutive polls (15 s apart) populate
// `_positionHistory` with bracketing fixes; `_deadReckon` interpolates and
// the globe re-renders the moving aircraft every frame.
//
// This probe captures the canvas twice (t0 and t=+10s) and compares. If the
// pixel diff is non-trivial (≥0.5% of pixels changed), the globe is
// re-rendering → aircraft are moving → fix verified. Companion data-plane
// check (curl /globe/aircraft before/after) attributes motion to upstream
// data updates vs. dead-reckoning from cached positions.
//
// Invoked from /home/zou/IntelHub inside the node:22-trixie docker container
// (`scripts/run-probe-motion.sh`) so console/node_modules/playwright is
// resolvable without installing node on the host.

import { chromium } from "playwright";
import { PNG } from "pngjs";
import fs from "node:fs";
import path from "node:path";

const URL = process.env.PROBE_URL || "http://10.10.10.35:8800/";
const OUT_DIR = process.env.PROBE_OUT || "/tmp/probe-motion";
const WAIT_MS_INITIAL = Number(process.env.PROBE_WAIT_INITIAL || 30_000);
const WAIT_MS_BETWEEN = Number(process.env.PROBE_WAIT_BETWEEN || 10_000);
const VIEWPORT = { width: 1440, height: 900 };

fs.mkdirSync(OUT_DIR, { recursive: true });

console.log(`[probe-motion] url=${URL} out=${OUT_DIR}`);
console.log(`[probe-motion] waits: initial=${WAIT_MS_INITIAL}ms between=${WAIT_MS_BETWEEN}ms`);

const browser = await chromium.launch({
  headless: true,
  args: [
    "--no-sandbox",
    "--disable-dev-shm-usage",
    // Cesium requires WebGL; headless chromium needs SwiftShader as the
    // GL backend. probe-gev.mjs:1484 documents the same SwiftShader-missing
    // warning. Without this flag the canvas exists but stays black.
    "--use-gl=angle",
    "--use-angle=swiftshader",
    "--enable-features=Vulkan",
    "--ignore-gpu-blocklist",
    "--enable-unsafe-swiftshader",
  ],
});
console.log(`[probe-motion] chromium launched`);

const ctx = await browser.newContext({
  viewport: VIEWPORT,
  deviceScaleFactor: 1,
});
const page = await ctx.newPage();

const consoleErrors = [];
page.on("pageerror", (e) => consoleErrors.push(`pageerror: ${e.message}`));
page.on("console", (msg) => {
  if (msg.type() === "error") consoleErrors.push(`console.error: ${msg.text()}`);
});

console.log(`[probe-motion] loading ${URL}`);
await page.goto(URL, { waitUntil: "networkidle", timeout: 120_000 });

// Auth gate: console SPA prompts for the agent API key (ihk_<hex>) before
// mounting any route's heavy machinery (Cesium globe, monitoring tables...).
// Headless runs start with a fresh browser context (no localStorage), so
// authenticate via the visible input + Connect button if present.
const apiKey = process.env.PROBE_API_KEY;
if (apiKey) {
  try {
    const input = await page.waitForSelector(
      'input[placeholder^="ihk_"], input[type="password"]',
      { timeout: 5_000 },
    );
    if (input) {
      await input.fill(apiKey);
      console.log(`[probe-motion] filled API key into auth input`);
      const connectBtn = await page.$('button:has-text("Connect")')
        ?? await page.$('button[type="submit"]');
      if (connectBtn) {
        await connectBtn.click();
        console.log(`[probe-motion] clicked Connect; waiting for re-render`);
        // Give the SPA time to navigate to the requested route + mount Cesium
        await page.waitForLoadState("networkidle", { timeout: 30_000 });
        await page.waitForTimeout(2_000);
      }
    }
  } catch (e) {
    console.log(`[probe-motion] no auth gate visible (${e.message.split("\n")[0]})`);
  }
} else {
  console.log(`[probe-motion] PROBE_API_KEY not set — assuming auth already satisfied (browser context reuse or localStorage seed)`);
}

// Diagnostic: capture the page state so a failure here is attributable.
// React render errors can unmount #root entirely (AGENTS.md "React render 崩
// 会变全黑" lesson); check #root + body innerText length and dump headless-
// mode clues before we trip over the canvas gate below.
const diag = await page.evaluate(() => {
  const root = document.getElementById("root");
  const canvas = document.querySelector("canvas");
  return {
    title: document.title,
    rootInnerLen: root ? root.innerHTML.length : -1,
    rootChildCount: root ? root.children.length : -1,
    bodyTextLen: document.body.innerText.length,
    canvasFound: !!canvas,
    canvasW: canvas ? canvas.width : 0,
    canvasH: canvas ? canvas.height : 0,
    webgl: (() => {
      const c = document.createElement("canvas");
      return !!(c.getContext("webgl2") || c.getContext("webgl"));
    })(),
    loaderStatus: document.querySelector("#loading-screen .loader-status")?.textContent ?? null,
  };
}).catch((e) => ({ error: String(e) }));
console.log(`[probe-motion] diag: ${JSON.stringify(diag)}`);
if (consoleErrors.length) {
  console.log(`[probe-motion] console_errors so far (${consoleErrors.length}):`);
  for (const e of consoleErrors.slice(0, 8)) console.log(`  ${e}`);
}

// Wait for the Cesium canvas to mount. probe-gev.mjs uses a similar gate.
try {
  await page.waitForSelector("canvas", { timeout: 60_000 });
} catch (err) {
  await page.screenshot({ path: path.join(OUT_DIR, "diag-no-canvas.png") });
  console.log(`[probe-motion] canvas never appeared; diagnostic screenshot: ${OUT_DIR}/diag-no-canvas.png`);
  throw err;
}
console.log(`[probe-motion] canvas found; waiting ${WAIT_MS_INITIAL / 1000}s for first poll cycle`);

await page.waitForTimeout(WAIT_MS_INITIAL);

// Cesium calls `holdContinuousRender('flights')` so the canvas keeps
// re-rendering every frame; both `page.screenshot` and `locator.screenshot`
// wait for element stability and stall. Bypass via CDP `Page.captureScreenshot`
// which captures the current frame without any settle check.
async function snap(p, file) {
  const session = await p.context().newCDPSession(p);
  const result = await session.send("Page.captureScreenshot", {
    format: "png",
    captureBeyondViewport: false,
  });
  await session.detach().catch(() => {});
  fs.writeFileSync(file, Buffer.from(result.data, "base64"));
}

const t0Path = path.join(OUT_DIR, "globe-t0.png");
await snap(page, t0Path);
console.log(`[probe-motion] t0 screenshot: ${t0Path}`);

await page.waitForTimeout(WAIT_MS_BETWEEN);

const t1Path = path.join(OUT_DIR, "globe-t1.png");
await snap(page, t1Path);
console.log(`[probe-motion] t1 screenshot: ${t1Path}`);

await browser.close();

// PNG byte-level compare + pixel diff.
const t0 = fs.readFileSync(t0Path);
const t1 = fs.readFileSync(t1Path);
const byteIdentical = t0.equals(t1);

const img0 = PNG.sync.read(t0);
const img1 = PNG.sync.read(t1);
const totalPx = img0.width * img0.height;
let diffPixels = 0;
for (let i = 0; i < img0.data.length; i += 4) {
  const dr = Math.abs(img0.data[i] - img1.data[i]);
  const dg = Math.abs(img0.data[i + 1] - img1.data[i + 1]);
  const db = Math.abs(img0.data[i + 2] - img1.data[i + 2]);
  // Threshold >5 per channel: tolerates sub-pixel jitter (Cesium's request
  // render mode may still nudge the canvas on geometry transform even when
  // no aircraft moved), but picks up the per-frame billboard redraws when
  // aircraft DO move.
  if (dr > 5 || dg > 5 || db > 5) diffPixels++;
}
const diffPct = ((diffPixels / totalPx) * 100).toFixed(3);

console.log(`\n=== probe-motion result ===`);
console.log(`byte_identical: ${byteIdentical}`);
console.log(`viewport: ${img0.width}x${img0.height}`);
console.log(`pixels_with_chan_delta_gt_5: ${diffPixels} / ${totalPx} (${diffPct}%)`);
console.log(`console_errors: ${consoleErrors.length}`);
for (const e of consoleErrors.slice(0, 5)) console.log(`  ${e}`);

let verdict;
let exitCode;
if (byteIdentical) {
  verdict = "STATIC — globe did not change between t0 and t+10s";
  exitCode = 1;
} else if (diffPct < 0.05) {
  verdict = "ESSENTIALLY STATIC — negligible diff (CSS/animation only, no Cesium re-render)";
  exitCode = 2;
} else if (diffPct < 0.5) {
  verdict = `MINIMAL MOTION — ${diffPct}% (motion present but minor; check upstream 429 rate limit)`;
  exitCode = 3;
} else {
  verdict = `MOTION VERIFIED — ${diffPct}% pixels changed across 10s window`;
  exitCode = 0;
}
console.log(`\n${verdict}`);
process.exit(exitCode);