#!/usr/bin/env node
// probe-globe.mjs — headless smoke for /globe (probe-monitor.mjs pattern).
// Usage: node console/probe-globe.mjs <base-url> <console-api-key>
import { chromium } from "playwright";

const [base, key] = process.argv.slice(2);
if (!base || !key) {
  console.error("usage: probe-globe.mjs <base-url> <console-api-key>");
  process.exit(2);
}

const errors = [];
const browser = await chromium.launch();
const ctx = await browser.newContext();
await ctx.addInitScript((k) => localStorage.setItem("intelhub.console.key", k), key);
const page = await ctx.newPage();
page.on("pageerror", (e) => errors.push(String(e)));
await page.goto(`${base}/globe`, { waitUntil: "load", timeout: 60_000 });
await page.waitForSelector("canvas", { timeout: 30_000 });
await page.waitForTimeout(9000); // basemap + first aircraft poll settle

const canvasOk = await page.evaluate(() => {
  const c = document.querySelector("canvas");
  return !!c && c.width > 0 && c.height > 0;
});
const acText = await page.textContent('[data-probe="aircraft-count"]').catch(() => null);
const satText = await page.textContent('[data-probe="sat-count"]').catch(() => null);
await browser.close();

const fatal = errors.filter((e) => !/ResizeObserver|GroupMarkerNotSet/i.test(e));
console.log(`canvas=${canvasOk} aircraft=${acText ?? "n/a"} satellites=${satText ?? "n/a"} pageerrors=${fatal.length}`);
if (fatal.length) {
  console.error(fatal.join("\n"));
  process.exit(1);
}
if (!canvasOk) {
  console.error("canvas missing or zero-size");
  process.exit(1);
}
if (acText !== null && Number.isNaN(parseInt(acText, 10))) {
  console.error("aircraft-count not numeric");
  process.exit(1);
}
console.log("probe-globe OK");
