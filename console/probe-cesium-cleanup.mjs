// Probe: confirm cesium cleanup works + no page errors during round-trip
import { chromium } from '/Volumes/TBU/Workspace/IntelHub/console/node_modules/playwright/index.mjs';

const BASE = process.env.BASE || 'http://10.10.10.35:8800';
const KEY = process.env.KEY;
if (!KEY) { console.error('KEY env var required'); process.exit(2); }

const browser = await chromium.launch({ headless: true });
const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
await ctx.addInitScript((key) => {
  localStorage.setItem('intelhub.console.key', key);
}, KEY);

const page = await ctx.newPage();
const errors = [];
const warnings = [];
page.on('pageerror', (e) => errors.push(`pageerror: ${e.message}`));
page.on('console', (m) => {
  if (m.type() === 'error') errors.push(`console.error: ${m.text().slice(0, 300)}`);
  if (m.type() === 'warning') warnings.push(`console.warn: ${m.text().slice(0, 200)}`);
});

async function nav(target) {
  await page.evaluate((t) => {
    window.history.pushState({}, '', t);
    window.dispatchEvent(new PopStateEvent('popstate'));
  }, target);
}

console.log('1. SPA boot');
await page.goto(`${BASE}/`, { waitUntil: 'domcontentloaded' });
await page.waitForTimeout(1000);

console.log('2. → /globe (1st visit)');
await nav('/globe');
await page.waitForSelector('#cesiumContainer canvas', { timeout: 60000 });
await page.waitForSelector('#cesium-credits', { state: 'attached', timeout: 10000 });
await page.waitForTimeout(3000);

console.log('3. → /radar (route leave)');
await nav('/radar');
await page.waitForTimeout(3000);

console.log('4. → /globe (2nd visit, re-entry)');
await nav('/globe');
await page.waitForSelector('#cesiumContainer canvas', { timeout: 60000 });
await page.waitForSelector('#cesium-credits', { state: 'attached', timeout: 10000 });
await page.waitForTimeout(5000);

// Check that HUD bars are populated on 2nd visit (means start() resolved)
const railPresent = await page.locator('[data-hud="left"] > *').count();
const bottomPresent = await page.locator('[data-hud="bottom"] > *').count();
console.log(`   layer rail on 2nd visit: ${railPresent > 0 ? 'YES' : 'NO'}`);
console.log(`   bottom bar on 2nd visit: ${bottomPresent > 0 ? 'YES' : 'NO'}`);

console.log('5. → /signals (route leave again)');
await nav('/signals');
await page.waitForTimeout(3000);

const creditsFinal = await page.locator('#cesium-credits').count();
const canvasFinal = await page.locator('#cesiumContainer canvas').count();
console.log(`   #cesium-credits: ${creditsFinal > 0 ? 'LEAK' : 'clean'}`);
console.log(`   #cesiumContainer canvas: ${canvasFinal > 0 ? 'LEAK' : 'clean'}`);

console.log('\n=== pageerrors ===');
if (errors.length === 0) console.log('  (none)');
errors.forEach((e) => console.log('  ', e));

console.log('\n=== warnings (first 5) ===');
warnings.slice(0, 5).forEach((w) => console.log('  ', w));

const ok = errors.length === 0 && railPresent > 0 && creditsFinal === 0 && canvasFinal === 0;
console.log(`\nVERDICT: ${ok ? 'PASS' : 'FAIL'}`);
await browser.close();
process.exit(ok ? 0 : 1);
