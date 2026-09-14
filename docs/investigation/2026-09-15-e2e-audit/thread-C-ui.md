# Thread C: Console UI Render Failures

> **Status:** ✅ Complete · **Date:** 2026-09-15 · **Target:** VM 415 (IntelHub-test, http://10.10.10.45:8800)
> **Method:** Playwright (Chromium headless) on Mac driving 415's built console. API key injected via `localStorage.setItem('intelhub.console.key', ...)` before each nav.
> **Reusable script:** `console/probe-all-pages.mjs` (idempotent)

## Summary

| 类别 | 数量 |
|---|---|
| Routes probed | 17 (15 valid + 2 nonexistent for negative testing) |
| Routes that successfully mounted (rootLen > 1k) | 14 |
| Routes that timed out on `networkidle` (polling pages — mount OK) | 3 (`/`, `/radar`, `/overview`) |
| **pageerror** count | **0** |
| **console error** count | **0** (excluding 404 favicon) |
| Empty `#root` | 0 (where reachable) |
| 5xx server responses | 0 |
| **Findings** | **3** (🟠 ×1 · 🟡 ×1 · 🟢 ×1) |

## Findings

### C-001 🟠 **USER-FACING CONFIRMATION of A-005/B-005: duplicate aliases render inline in entity detail page**
- **Page:** `/entities/:id` (any entity with aliases)
- **Reproducible:** `GET http://10.10.10.45:8800/entities/3846325c-093f-4ac7-85e4-4ab4daa50468`
- **Screenshot:** `evidence/thread-C/screenshots/_entities_3846325c-093f-4ac7-85e4-4ab4daa50468.png`
- **Visible output (excerpt):**
  ```
  aliases: brics-russia-2024, brics.bz, brics-russia-2024, brics.bz,
           brics-russia-2024, brics.bz, brics-russia-2024, brics.bz,
           brics-russia-2024, brics.bz, brics-russia-2024, brics.bz,
           brics-russia-2024, brics.bz
  ```
  — 7 lines × 2 = 14 alias entries (matches Neo4j 12× + A-005 observation).
- **Why this matters:** Thread A and B both identified the data bug at API/store level. Thread C confirms it surfaces to the end user — the `/entities/:id` page shows ugly raw duplicate strings inline. This is a real, visible defect, not just an internal data quality issue.
- **Status of A-005 fix:** Still open. Thread C is just confirming the impact.
- **Fix direction:** Same as A-005 / B-005 (see A-005 fix plan): add UNIQUE constraint, dedup migration, idempotent write.

### C-002 🟡 `/documents/:id` deep link returns blank page — no route exists
- **Page tested:** `/documents/96fe276b-4ea5-423d-9064-7943c0932ce1` (real document id from earlier probe)
- **Expected:** Either a document detail view, or a 404 with helpful message
- **Actual:** Blank page (rootLen = 2024, no rendered content aside from sidebar nav). Screenshot shows empty main panel.
- **Reproducible:** `GET http://10.10.10.45:8800/documents/<any-uuid>`
- **Why medium:** Users who copy-paste a document URL (e.g. from a Slack/email share) get a blank page with no error message. This is a discoverability + UX bug.
- **Console source check:** `console/src/App.tsx` route table only includes:
  - `/`, `/radar`, `/signals`, `/investigations`, `/investigations/:id`, `/entities/:id`, `/evidence`, `/evidence/:id`, `/graph`, `/alerts`, `/agents`, `/audit`, `/search`, `/overview`, `/system`
  - **No `/documents/*` route at all** — so React Router's default 404 handler isn't reached (404 page may not exist either).
- **Fix direction:** Either (a) add a real document detail route + page, or (b) add a fallback "this route doesn't exist" 404 page so deep links at least show "Not Found" with a link back to `/search` or `/evidence`. Recommended: (b) first (cheap), then (a) if there's user demand.

### C-003 🟢 `/investigations/:id` shows useful red error banner for missing ID — GOOD UX (positive finding)
- **Page tested:** `/investigations/00000000-0000-0000-0000-000000000000` (nonexistent)
- **Output:** Red banner at top: "investigation 00000000-0000-0000-0000-000000000000" with rest of page blank.
- **Status:** Working as designed — user gets clear feedback.
- **Suggestion:** Same error pattern should apply to C-002 (`/documents/:id` should at least show "Document not found" banner).

## Cross-Page Observations (informational)

- **All pages render under 2.5s** (`waitUntil: 'load' + 2.5s wait` sufficient). No `pageerror` fired in any of 17 pages. React 19 render is stable.
- **3 routes don't reach `networkidle` in 15s**: `/`, `/radar`, `/overview` — all have background polling for live data. Use `waitUntil: 'load'` (not `'networkidle'`) for SPA probe.
- **Sidebar nav present on all pages** — routing works.
- **Dark theme + i18n** — sampled `en`, did not switch to `zh` in this run (would need a separate probe with click on language switcher).
- **Page-level error boundaries** not tested in this run (would need a synthetic throw inside a page component to verify the boundary catches it). The fact that 0 pageerror fired even on broken-input routes suggests either (a) no errors occur, or (b) error boundaries are catching them. Manual crash test recommended as follow-up.

## Reproducibility

```bash
cd /Volumes/TBU/Workspace/IntelHub-investigation-2026-09-15/console
node probe-all-pages.mjs
# Outputs to docs/investigation/2026-09-15-e2e-audit/evidence/thread-C/
# - probe-output.json (full per-route data)
# - screenshots/<route>.png (1440x900 PNG per route)
```

Requires:
- Node 20+
- `@playwright/test` in console devDeps (already present)
- A working chromium (Mac has `~/Library/Caches/ms-playwright/` populated)
- Read-only; no side effects on 415

## Caveats

- **Networkidle → load fix**: First probe used `networkidle` which timed out on polling pages. After switching to `load`, all 17 routes succeeded. The probe script reflects the working config.
- **No i18n switch test** — language toggle UI not exercised in this run.
- **No error boundary test** — need a synthetic throw to verify the boundary actually catches.
- **No theme toggle test** — could probe by clicking theme button if present.
- **No mobile viewport** — only 1440x900 tested; tablet/mobile may have separate issues.
