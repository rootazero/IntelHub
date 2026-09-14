// Thread C probe — drive each console route, capture pageerror + screenshot.
// Usage: node probe-all-pages.mjs
// Reuses /Users/zouguojun/Library/Caches/ms-playwright/ if installed.
import { chromium } from '@playwright/test';
import fs from 'fs';
import path from 'path';

const BASE = process.env.BASE || 'http://10.10.10.45:8800';
const KEY = process.env.KEY || fs.readFileSync('/tmp/inv-e2e/key.txt', 'utf8').trim();
const OUT = '/Volumes/TBU/Workspace/IntelHub-investigation-2026-09-15/docs/investigation/2026-09-15-e2e-audit/evidence/thread-C';
fs.mkdirSync(OUT, { recursive: true });
fs.mkdirSync(`${OUT}/screenshots`, { recursive: true });

// All routes from console/src/App.tsx (or wherever the router is)
const ROUTES = [
  '/', '/radar', '/signals', '/alerts', '/investigations', '/investigations/1513e6eb-c1f0-4a46-b78e-9b7741e833f5',
  '/entities/3846325c-093f-4ac7-85e4-4ab4daa50468', '/evidence', '/agents', '/audit', '/graph',
  '/search', '/overview', '/system', '/documents/96fe276b-4ea5-423d-9064-7943c0932ce1',
  '/investigations/00000000-0000-0000-0000-000000000000', // nonexistent
  '/entities/00000000-0000-0000-0000-000000000000', // nonexistent
];

const results = [];
const errors = [];
const pageErrors = [];

const browser = await chromium.launch({ headless: true });
const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });

// Inject API key into localStorage before each page (most consoles read from there)
// Real key name from console/src/api.ts and pages/System.tsx: 'intelhub.console.key'
await ctx.addInitScript((key) => {
  try {
    localStorage.setItem('intelhub.console.key', key);
  } catch (e) {}
}, KEY);

for (const route of ROUTES) {
  const page = await ctx.newPage();
  const pageErr = [];
  const consoleErr = [];
  const requestFail = [];

  page.on('pageerror', (e) => pageErr.push({ msg: String(e), stack: e.stack?.slice(0, 800) }));
  page.on('console', (msg) => {
    if (msg.type() === 'error') {
      const text = msg.text();
      // Filter known noise
      if (text.includes('favicon')) return;
      if (text.includes('Failed to load resource') && text.includes('404')) return;
      consoleErr.push(text);
    }
  });
  page.on('requestfailed', (req) => {
    requestFail.push({ url: req.url(), failure: req.failure()?.errorText });
  });

  const url = BASE + route;
  const safeName = route === '/' ? '_root' : route.replace(/\//g, '_');
  const screenshotPath = `${OUT}/screenshots/${safeName}.png`;
  let status = null;
  let rootLen = null;

  try {
    const resp = await page.goto(url, { waitUntil: 'load', timeout: 15000 }).catch((e) => ({ _navErr: String(e) }));
    if (resp && resp.status) status = resp.status;
    if (resp && resp._navErr) status = `NAV_ERR: ${resp._navErr}`;
    // Wait extra for async renders / Leaflet / polling
    await page.waitForTimeout(2500);
    rootLen = await page.$eval('#root', el => el.innerHTML.length).catch(() => null);
    try {
      await page.screenshot({ path: screenshotPath, fullPage: false, timeout: 5000 });
    } catch (e) {
      // ignore — just continue
    }
  } catch (e) {
    status = `EXC: ${String(e).slice(0, 200)}`;
  }

  results.push({
    route, url, status, rootLen, screenshot: screenshotPath.replace(OUT, '.'),
    pageErrors: pageErr, consoleErrors: consoleErr, requestFailures: requestFail,
  });

  if (pageErr.length || consoleErr.length) {
    pageErrors.push({ route, pageErr, consoleErr });
  }

  await page.close();
}

await browser.close();

// Summarize
const summary = {
  routes_tested: results.length,
  routes_with_pageerrors: results.filter(r => r.pageErrors.length > 0).length,
  routes_with_consoleerrors: results.filter(r => r.consoleErrors.length > 0).length,
  routes_with_empty_root: results.filter(r => r.rootLen === 0 || r.rootLen === null).length,
  routes_5xx: results.filter(r => typeof r.status === 'number' && r.status >= 500).length,
  routes_nav_failed: results.filter(r => typeof r.status === 'string' && r.status.startsWith('NAV_ERR')).length,
};

fs.writeFileSync(`${OUT}/probe-output.json`, JSON.stringify({ summary, results, pageErrors }, null, 2));
console.log('=== Thread C probe summary ===');
console.log(JSON.stringify(summary, null, 2));
console.log(`\nDetails: ${OUT}/probe-output.json`);
