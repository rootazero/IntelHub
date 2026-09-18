// Focused perf probe — load Radar, slide mouse across the map area, measure
// rAF deltas during the slide. Used as the failing-test-case + post-fix
// verification for the DOM-marker → GeoJSON circle-layer refactor.
//
// Usage:
//   KEY=$(ssh -o BatchMode=yes Debian-test 'grep api_key: /home/zou/IntelHub/core/agent-keys.txt | head -1 | grep -o ihk_[a-f0-9]*')
//   BASE=http://10.10.10.35:8800 node console/probe-radar-marker-perf.mjs
//
// Output: one JSON line with p50/p95/max frame ms + total DOM markers observed.
// Interpretation:
//   p95 < 18ms (~55fps headroom) and max < 50ms → acceptable
//   p95 > 33ms or max > 100ms → still laggy
//
// Pre-fix baseline: ~1213 DOM .radar-marker elements, p95 ~30-60ms during slide.
// Post-fix target:  0 .radar-marker elements (canvas-rendered), p95 < 18ms.
import { chromium } from '@playwright/test';

const BASE = process.env.BASE || 'http://10.10.10.45:8800';
const KEY = process.env.KEY;

if (!KEY) {
  console.error('KEY env var required (ihk_...)');
  process.exit(2);
}

// Allow override of the chromium binary path so this probe works with
// whatever playwright build the caller's node_modules carries (Aleph-Hub
// in our case uses 1243). Headless=false gives real GPU compositing
// numbers matching the user's browser — but only works on a host with
// a display. Default: headless (still useful for DOM count regression).
const launchOpts = { headless: process.env.HEADFUL !== '1' };
if (process.env.CHROMIUM_PATH) launchOpts.executablePath = process.env.CHROMIUM_PATH;
const browser = await chromium.launch(launchOpts);
const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
await ctx.addInitScript((key) => {
  try { localStorage.setItem('intelhub.console.key', key); } catch (e) {}
}, KEY);

const page = await ctx.newPage();
const pageErrors = [];
page.on('pageerror', (e) => pageErrors.push(String(e)));

await page.goto(`${BASE}/radar`, { waitUntil: 'domcontentloaded', timeout: 30000 });

// Wait for the map style to finish loading AND for the first batch of
// events to be drawn. Vite + MapLibre style fetch + first API round-trip
// + first WebGL frame usually lands inside 10s; give it 12s to be safe.
await page.waitForFunction(
  () => {
    const m = document.querySelector('.maplibregl-map');
    if (!m) return false;
    // maplibregl attaches the canvas once style is loaded
    return !!m.querySelector('canvas.maplibregl-canvas');
  },
  { timeout: 20000 },
).catch(() => {});
await page.waitForTimeout(8000);

// Probe both metrics: DOM marker count (in-page) + frame timing during
// REAL mouse sweeps (Playwright's mouse.move dispatches native browser
// input so the browser does actual hit-testing against DOM markers).
const domInfo = await page.evaluate(() => {
  const domMarkers = document.querySelectorAll('.radar-marker, .hud-marker').length;
  const wrap = document.querySelector('.hud-map-wrap');
  if (wrap) return { domMarkers, wrap: 'hud-map-wrap' };
  const rel = Array.from(document.querySelectorAll('div')).find(
    (d) => typeof d.className === 'string' && d.className.includes('relative min-w-0 flex-1') && d.querySelector('.maplibregl-canvas-container'),
  );
  if (rel) return { domMarkers, wrap: 'radar-rel' };
  const m = document.querySelector('.maplibregl-canvas-container');
  return { domMarkers, wrap: m ? 'maplibregl-canvas' : null };
});

// Install an in-page rAF timer to record frame deltas while we move the
// real mouse. Sampled out-of-band by page.evaluate after the sweep.
await page.evaluate(() => {
  window.__rafDeltas = [];
  window.__rafLast = performance.now();
  window.__rafStop = false;
  function tick() {
    if (window.__rafStop) return;
    const now = performance.now();
    const d = now - window.__rafLast;
    if (d < 500) window.__rafDeltas.push(d); // skip long pauses
    window.__rafLast = now;
    requestAnimationFrame(tick);
  }
  requestAnimationFrame(tick);
});

if (!domInfo.wrap) {
  console.log(JSON.stringify({ error: 'no map container found', domInfo, pageErrors }, null, 2));
  await browser.close();
  process.exit(1);
}

const box = await page.locator(`.${domInfo.wrap === 'hud-map-wrap' ? 'hud-map-wrap' : 'maplibregl-canvas-container'}`).first().boundingBox();
if (!box) {
  console.log(JSON.stringify({ error: 'no bbox', domInfo, pageErrors }, null, 2));
  await browser.close();
  process.exit(1);
}

// Real mouse sweeps across the map. Slow horizontal passes trigger every
// marker :hover transition along the way — exactly the user complaint.
const cx = box.x + box.width / 2;
const cy = box.y + box.height / 2;
const x1 = box.x + 20;
const x2 = box.x + box.width - 20;
for (let sweep = 0; sweep < 5; sweep++) {
  await page.mouse.move(x1, cy, { steps: 1 });
  await page.mouse.move(x2, cy, { steps: 80 }); // ~30px steps across the map
  await page.mouse.move(x1, cy, { steps: 80 });
}

const rafStats = await page.evaluate(() => {
  window.__rafStop = true;
  const arr = window.__rafDeltas;
  arr.sort((a, b) => a - b);
  const pick = (p) => arr[Math.floor((arr.length - 1) * p)] ?? null;
  return { samples: arr.length, p50: pick(0.50), p95: pick(0.95), p99: pick(0.99), max: arr[arr.length - 1] ?? null };
});

console.log(JSON.stringify({ ...domInfo, ...rafStats, pageErrors }, null, 2));

console.log(JSON.stringify({ ...domInfo, ...rafStats, pageErrors }, null, 2));
await browser.close();
