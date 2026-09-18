// Probe: confirm cesium credit icon persists on /globe → back navigation.
// Uses CLIENT-SIDE navigation (React Router pushState) — full page reload
// wouldn't reproduce the leak because React would tear down on its own.
//
// Expected after fix: #cesium-credits is removed from document.body
// when GlobeV2 unmounts (client-side route switch).
import { chromium } from '/Volumes/TBU/Workspace/IntelHub/console/node_modules/playwright/index.mjs';

const BASE = process.env.BASE || 'http://10.10.10.35:8800';
const KEY = process.env.KEY;
if (!KEY) {
  console.error('KEY env var required');
  process.exit(2);
}

const browser = await chromium.launch({ headless: true });
const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
await ctx.addInitScript((key) => {
  localStorage.setItem('intelhub.console.key', key);
}, KEY);

const page = await ctx.newPage();
const errors = [];
page.on('pageerror', (e) => errors.push(`pageerror: ${e.message}`));
page.on('console', (m) => {
  if (m.type() === 'error') errors.push(`console.error: ${m.text().slice(0, 200)}`);
});

console.log('1. Navigate to / (full page load to bootstrap SPA)');
await page.goto(`${BASE}/`, { waitUntil: 'domcontentloaded' });
await page.waitForTimeout(1000);

console.log('2. Click "Globe" navlink (client-side route switch)');
await page.locator('a[href="/globe"]').first().click({ force: true });
console.log('3. Wait for cesium boot...');
await page.waitForSelector('#cesiumContainer canvas', { timeout: 60000 });
await page.waitForSelector('#cesium-credits', { state: 'attached', timeout: 10000 });
await page.waitForTimeout(2000);

const creditsOnGlobe = await page.locator('#cesium-credits').count();
const canvasOnGlobe = await page.locator('#cesiumContainer canvas').count();
const creditsInfo = await page.evaluate(() => {
  const el = document.getElementById('cesium-credits');
  if (!el) return null;
  const r = el.getBoundingClientRect();
  const cs = window.getComputedStyle(el);
  return { width: r.width, height: r.height, position: cs.position, left: cs.left, bottom: cs.bottom };
});
console.log(`   #cesium-credits present: ${creditsOnGlobe > 0 ? 'YES' : 'NO'}`);
console.log(`   #cesium-credits info: ${JSON.stringify(creditsInfo)}`);
console.log(`   #cesiumContainer canvas present: ${canvasOnGlobe > 0 ? 'YES' : 'NO'}`);

await page.screenshot({ path: '/tmp/probe-globe-loaded.png', fullPage: false });

console.log('4. Click "Radar" navlink (client-side away from /globe)');
// cesium canvas intercepts pointer events — use programmatic nav to avoid
// the canvas eating the click. React Router picks up history.pushState.
await page.evaluate(() => { window.history.pushState({}, '', '/radar'); window.dispatchEvent(new PopStateEvent('popstate')); });
await page.waitForTimeout(3000);

const creditsAfter = await page.locator('#cesium-credits').count();
const cesiumContainerAfter = await page.locator('#cesiumContainer').count();
const cesiumCanvasAnywhere = await page.evaluate(() => {
  return Array.from(document.querySelectorAll('canvas'))
    .filter(c => c.closest('.cesium-widget') || c.closest('#cesiumContainer'))
    .map(c => ({ parent: c.parentElement?.id, visible: c.offsetParent !== null, w: c.width, h: c.height }));
});
console.log(`   #cesium-credits still on page: ${creditsAfter > 0 ? 'YES (BUG)' : 'NO (clean)'}`);
console.log(`   #cesiumContainer still on page: ${cesiumContainerAfter > 0 ? 'YES (engine container leaked)' : 'NO (clean)'}`);
console.log(`   cesium canvases still in DOM: ${JSON.stringify(cesiumCanvasAnywhere)}`);

await page.screenshot({ path: '/tmp/probe-radar-after.png', fullPage: false });

console.log('5. Navigate back to /globe via navlink');
await page.evaluate(() => { window.history.pushState({}, '', '/globe'); window.dispatchEvent(new PopStateEvent('popstate')); });
await page.waitForTimeout(5000);
const creditsOnReturn = await page.locator('#cesium-credits').count();
const canvasOnReturn = await page.locator('#cesiumContainer canvas').count();
console.log(`   #cesium-credits on return: ${creditsOnReturn > 0 ? 'YES' : 'NO'}`);
console.log(`   #cesiumContainer canvas on return: ${canvasOnReturn > 0 ? 'YES' : 'NO'}`);

await page.screenshot({ path: '/tmp/probe-globe-return.png', fullPage: false });

if (errors.length) {
  console.log('\nPage errors:');
  errors.slice(0, 10).forEach((e) => console.log('  ', e));
}

const ok = creditsAfter === 0 && cesiumContainerAfter === 0;
console.log(`\nVERDICT: ${ok ? 'PASS (no leak)' : 'FAIL (leak detected)'}`);

await browser.close();
process.exit(ok ? 0 : 1);
