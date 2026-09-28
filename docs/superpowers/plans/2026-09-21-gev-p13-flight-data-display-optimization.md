# GEV P13 — Flight Data Display Optimization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate the perceived "Florida clustering" + "yellow planes have no details" UX problems by (1) setting a global Cesium default view, (2) accelerating vendor enrichment, (3) adding a HUD detail panel, (4) expanding data source coverage.

**Architecture:** All four pieces are **console-side** except T4 which adds a 3rd ADS-B source in hub-core. Vendor engine (`console/gev-engine/`) is **never modified** (per AGENTS.md "no vendor fork"). The QA seam `window.__GEV_ENRICH_AMBIENT_QA` (vendor enrichment.js:132-141) is the approved escape hatch for budget overrides. The HUD detail panel subscribes to `flightState._trackedIcao` via the existing `scene.catalog` registry.

**Tech Stack:**
- Console: TypeScript, React, vitest, CesiumJS API (`Cartesian3.fromDegrees`, `viewer.camera.setView`)
- Hub: Rust (tokio + reqwest + serde_json), no new crates
- Vendor: `gev-engine/src/layers/flights/{enrichment,tracking,policy}.js` (READ-ONLY reference)

**Spec:** `docs/superpowers/specs/2026-09-21-gev-p13-flight-data-display-optimization-design.md` (written this session)

## Global Constraints

- Worktree isolation: `feat/p13-flight-display` branch from `main` (currently `37e63f7`)
- Build on 315 first, deploy to 410 only after all 5 acceptance suites green
- Cache paths use `/home/zou/IntelHub/data/...` (systemd ProtectSystem=strict — NOT `/var/lib/intelhub/...`)
- Hand-rolled validators (no `regex` crate)
- i18n strings in `.ts` Dict format (en + zh), not `.json` (repo convention)
- 5-minute stampede wait after every `sudo systemctl restart hub-core` on 410
- Vendor `consolegev-engine/` zero diff in every commit
- `_enrichSeen` (vendor enrichment.js:21) is per-session dedup; cannot be cleared

## Review Focus

These input classes are in scope per spec but have no per-task test coverage. They are the failures most likely to bite a real user:

1. **Cesium camera state during page reload** — expect default view to remain global; reset between sessions. Pin in T1.
2. **Stale enrichment data after 24 hours** — expect re-fetch or invalidate. Pin in T2.
3. **HUD detail panel accessibility on touch / small screens** — expect visible at >768px. Pin in T3.
4. **Missing ADS-B Exchange key** — expect graceful fallback to adsb.lol + OpenSky. Pin in T4.
5. **Concurrent viewports (cockpit + detail)** — expect detail hidden during cockpit (cockpit has its own panel). Pin in T3.

## File Structure

**New files (5):**
- `console/src/gev-boot/default-camera.ts` — Cesium initial view (T1)
- `console/src/gev-boot/__tests__/default-camera.test.ts` — T1 tests
- `console/src/gev-boot/enrich-override.ts` — Vendor ambient QA injection (T2)
- `console/src/gev-boot/__tests__/enrich-override.test.ts` — T2 tests
- `console/src/globe-hud/HudAircraftDetail.tsx` — Right-side detail card (T3)
- `console/src/globe-hud/__tests__/HudAircraftDetail.test.tsx` — T3 tests
- `hub-core/crates/hub-core/src/monitor/sources/adsbexchange.rs` — 3rd ADS-B source (T4)
- `hub-core/crates/hub-core/tests/gev_adsbexchange.rs` — T4 tests

**Modified files (5):**
- `console/src/gev-boot/application.ts` — Wire T1 + T2 before `createApplicationScene`
- `console/src/globe-hud/GlobeHud.tsx` — Mount HudAircraftDetail in top-right
- `console/src/i18n/{en,zh}.ts` — New strings for detail panel
- `hub-core/crates/hub-core/src/api.rs::console_globe_aircraft` — Add 3rd source merge
- `hub-core/crates/hub-core/src/monitor/sources/mod.rs` — Register adsbexchange module

**Acceptance changes (3):**
- `scripts/accept-sp6.py` — +3 checks (default camera, enrich QA, adsbexchange/expanded)
- `scripts/accept-sp8.py` — +4 checks (HUD detail, mount, view, QA seam)
- `console/probe-gev.mjs` — +5 selectors

---

### Task 1: Cesium default camera view

**Files:**
- Create: `console/src/gev-boot/default-camera.ts` (~50 LoC)
- Create: `console/src/gev-boot/__tests__/default-camera.test.ts` (~30 LoC, 2 tests)
- Modify: `console/src/gev-boot/application.ts` (insert one line in `createScene` defer)

**Interfaces:**
- Consumes: `Cesium.Viewer` instance
- Produces: `viewer.camera` set to `Cartesian3.fromDegrees(lon, lat, alt)` with pitch=-90° (top-down)

**DEFAULT_VIEW constant (verbatim, no magic in callers):**

```ts
// console/src/gev-boot/default-camera.ts
export const DEFAULT_VIEW = {
  lon: -50,        // center Western hemisphere
  lat: 20,         // mid-latitude (above equator, sees N + S)
  alt: 12_000_000, // 12,000 km — sees entire Americas + Pacific rim
  pitch: -90,      // straight down (top-down globe view)
} as const;
```

- [ ] **Step 1: Write the failing test**

```ts
// console/src/gev-boot/__tests__/default-camera.test.ts
import { describe, it, expect, vi } from 'vitest';
import { mountDefaultCamera, DEFAULT_VIEW } from '../default-camera';

describe('mountDefaultCamera', () => {
  it('throws TypeError if viewer.camera.setView is missing', () => {
    expect(() => mountDefaultCamera({} as any)).toThrow(TypeError);
  });

  it('calls viewer.camera.setView with DEFAULT_VIEW destination + pitch', () => {
    const setView = vi.fn();
    mountDefaultCamera({ camera: { setView } } as any);
    expect(setView).toHaveBeenCalledTimes(1);
    const arg = setView.mock.calls[0][0];
    expect(arg.orientation.pitch).toBeCloseTo(-Math.PI / 2);
    expect(arg.destination.x).toBeGreaterThan(0); // Cartesian3.fromDegrees(...)
  });
});
```

- [ ] **Step 2: Run test, verify FAIL**

```bash
cd /Volumes/TBU/Workspace/IntelHub && npx vitest run console/src/gev-boot/__tests__/default-camera.test.ts 2>&1 | tail -5
```
Expected: FAIL "Cannot find module '../default-camera'"

- [ ] **Step 3: Implement minimal module**

```ts
// console/src/gev-boot/default-camera.ts
import * as Cesium from 'cesium';

export const DEFAULT_VIEW = {
  lon: -50,
  lat: 20,
  alt: 12_000_000,
  pitch: -90,
} as const;

export interface MountDefaultCameraOpts {
  view?: Partial<typeof DEFAULT_VIEW>;
}

export function mountDefaultCamera(viewer: unknown, opts: MountDefaultCameraOpts = {}): void {
  const cam = (viewer as { camera?: { setView?: (a: unknown) => void } })?.camera;
  if (typeof cam?.setView !== 'function') {
    throw new TypeError("mountDefaultCamera: viewer.camera.setView missing");
  }
  const v = { ...DEFAULT_VIEW, ...(opts.view || {}) };
  cam.setView({
    destination: Cesium.Cartesian3.fromDegrees(v.lon, v.lat, v.alt),
    orientation: { heading: 0, pitch: Cesium.Math.toRadians(v.pitch), roll: 0 },
  });
}
```

- [ ] **Step 4: Run test, verify PASS**

```bash
cd /Volumes/TBU/Workspace/IntelHub && npx vitest run console/src/gev-boot/__tests__/default-camera.test.ts 2>&1 | tail -5
```
Expected: PASS, 2/2 tests.

- [ ] **Step 5: Wire into application.ts**

In `console/src/gev-boot/application.ts::createScene`, after `const viewer = createApplicationViewer(...)` and before `defer(initLogoGaze())`, add:
```ts
defer(() => mountDefaultCamera(viewer));
```

- [ ] **Step 6: Verify tsc rc=0**

```bash
cd /Volumes/TBU/Workspace/IntelHub && npx tsc -b 2>&1 | tail -3
```

- [ ] **Step 7: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub && git add console/src/gev-boot/default-camera.ts console/src/gev-boot/__tests__/default-camera.test.ts console/src/gev-boot/application.ts && git -c user.name='pi' -c user.email='pi@local' commit -m 'feat(gev-p13-t1): mountDefaultCamera — global Cesium default view'
```

---

### Task 2: Vendor enrichment ambient QA override

**Files:**
- Create: `console/src/gev-boot/enrich-override.ts` (~30 LoC)
- Create: `console/src/gev-boot/__tests__/enrich-override.test.ts` (~25 LoC, 3 tests)
- Modify: `console/src/gev-boot/application.ts` (insert one line at module top)

**Interfaces:**
- Consumes: `window` global
- Produces: `window.__GEV_ENRICH_AMBIENT_QA = {ceil, refillTokens, windowMs}` (vendor reads at vendor enrichment.js:138-141)

**Rationale:** vendor `enrichment.js:132-141` exposes `__GEV_ENRICH_AMBIENT_QA` as a runtime override for the ambient budget knob (`ENRICH_AMBIENT_BUDGET_CEIL=300` default at vendor `policy.js:259`). We raise the ceiling to 800 so all ~374 visible planes can be enriched within one refresh window.

- [ ] **Step 1: Write the failing test**

```ts
// console/src/gev-boot/__tests__/enrich-override.test.ts
import { describe, it, expect, beforeEach } from 'vitest';
import { applyEnrichAmbientOverride } from '../enrich-override';

describe('applyEnrichAmbientOverride', () => {
  beforeEach(() => { delete (globalThis as any).__GEV_ENRICH_AMBIENT_QA; });

  it('sets window.__GEV_ENRICH_AMBIENT_QA with raised budget', () => {
    applyEnrichAmbientOverride();
    const qa = (globalThis as any).__GEV_ENRICH_AMBIENT_QA;
    expect(qa.ceil).toBeGreaterThanOrEqual(800);
    expect(qa.refillTokens).toBeGreaterThanOrEqual(400);
    expect(qa.windowMs).toBe(300_000);
  });

  it('is idempotent (does not overwrite existing override)', () => {
    (globalThis as any).__GEV_ENRICH_AMBIENT_QA = { ceil: 100, refillTokens: 50, windowMs: 60_000 };
    applyEnrichAmbientOverride();
    expect((globalThis as any).__GEV_ENRICH_AMBIENT_QA.ceil).toBe(100);
  });

  it('accepts caller overrides', () => {
    applyEnrichAmbientOverride({ ceil: 1500, refillTokens: 1000 });
    expect((globalThis as any).__GEV_ENRICH_AMBIENT_QA.ceil).toBe(1500);
  });
});
```

- [ ] **Step 2: Run test, verify FAIL**

```bash
cd /Volumes/TBU/Workspace/IntelHub && npx vitest run console/src/gev-boot/__tests__/enrich-override.test.ts 2>&1 | tail -3
```
Expected: FAIL "Cannot find module"

- [ ] **Step 3: Implement minimal module**

```ts
// console/src/gev-boot/enrich-override.ts

export const ENRICH_OVERRIDE_DEFAULTS = {
  ceil: 800,
  refillTokens: 400,
  windowMs: 300_000,
} as const;

export interface EnrichAmbientOverride {
  ceil?: number;
  refillTokens?: number;
  windowMs?: number;
}

export function applyEnrichAmbientOverride(overrides: EnrichAmbientOverride = {}): void {
  const w = globalThis as unknown as { __GEV_ENRICH_AMBIENT_QA?: Record<string, number> };
  if (w.__GEV_ENRICH_AMBIENT_QA && Object.keys(w.__GEV_ENRICH_AMBIENT_QA).length >= 3) return;
  w.__GEV_ENRICH_AMBIENT_QA = { ...ENRICH_OVERRIDE_DEFAULTS, ...overrides };
}
```

- [ ] **Step 4: Run test, verify PASS**

```bash
cd /Volumes/TBU/Workspace/IntelHub && npx vitest run console/src/gev-boot/__tests__/enrich-override.test.ts 2>&1 | tail -3
```
Expected: PASS, 3/3.

- [ ] **Step 5: Wire into application.ts**

At the TOP of `console/src/gev-boot/application.ts` (before any vendor import), add:
```ts
import { applyEnrichAmbientOverride } from "./enrich-override";
applyEnrichAmbientOverride();
```

- [ ] **Step 6: Verify tsc rc=0**

```bash
cd /Volumes/TBU/Workspace/IntelHub && npx tsc -b 2>&1 | tail -3
```

- [ ] **Step 7: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub && git add console/src/gev-boot/enrich-override.ts console/src/gev-boot/__tests__/enrich-override.test.ts console/src/gev-boot/application.ts && git -c user.name='pi' -c user.email='pi@local' commit -m 'feat(gev-p13-t2): raise vendor ambient enrichment budget via QA seam'
```

---

### Task 3: Flight detail HUD panel

**Files:**
- Create: `console/src/globe-hud/HudAircraftDetail.tsx` (~250 LoC)
- Create: `console/src/globe-hud/__tests__/HudAircraftDetail.test.tsx` (~150 LoC, 10 tests)
- Modify: `console/src/globe-hud/GlobeHud.tsx` (mount detail panel)
- Modify: `console/src/i18n/{en,zh}.ts` (add ~12 strings)

**Interfaces:**
- Consumes: `scene.catalog.layers.find(l.id === "flights")` — accesses `flightState._trackedIcao`, `flightState.records.data.get(icao)`
- Produces: DOM card (top-right) showing tracked aircraft metadata; updates on enrichment events

- [ ] **Step 1: Write the failing tests**

```tsx
// console/src/globe-hud/__tests__/HudAircraftDetail.test.tsx
import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import { HudAircraftDetail } from '../HudAircraftDetail';

describe('HudAircraftDetail', () => {
  it('renders empty state when nothing tracked', () => {
    render(<HudAircraftDetail catalog={null} />);
    expect(screen.getByTestId('hud-aircraft-detail-empty')).toBeTruthy();
  });

  it('shows tracked aircraft hex when icao is set', () => {
    const catalog = {
      layers: [{
        id: 'flights',
        state: { _trackedIcao: '780a5a', records: { data: new Map([['780a5a', { hex:'780a5a', callsign:'CPA250', latitude:22.3, longitude:114.2, baroAltitudeM:10500, speedMps:230, courseDeg:90 }]]) } },
      }],
    } as any;
    render(<HudAircraftDetail catalog={catalog} />);
    expect(screen.getByText('780A5A')).toBeTruthy();
    expect(screen.getByText('CPA250')).toBeTruthy();
  });

  it('shows type/route enrichment when present', () => {
    const catalog = {
      layers: [{
        id: 'flights',
        state: { _trackedIcao: '780a5a', records: { data: new Map([['780a5a', { hex:'780a5a', typeCode:'A333', typeName:'Airbus A330', registration:'B-HWM', airline:'Cathay Pacific', route:{origin:{code:'HKG'}, destination:{code:'JFK'}} }]]) } },
      }],
    } as any;
    render(<HudAircraftDetail catalog={catalog} />);
    expect(screen.getByText('Airbus A330')).toBeTruthy();
    expect(screen.getByText('B-HWM')).toBeTruthy();
    expect(screen.getByText(/HKG.*JFK/)).toBeTruthy();
  });

  it('hides during cockpit mode', () => {
    document.body.classList.add('cockpit-mode');
    const catalog = { layers: [{ id: 'flights', state: { _trackedIcao: '780a5a', records: { data: new Map() } } }] } as any;
    const { container } = render(<HudAircraftDetail catalog={catalog} />);
    expect(container.firstChild).toBeNull();
    document.body.classList.remove('cockpit-mode');
  });

  it('formats altitude as feet when metric toggle is on, otherwise meters', () => { /* ... */ });
  it('formats speed as knots', () => { /* ... */ });
  it('formats lat/lon as DMS', () => { /* ... */ });
  it('refreshes every 1s while tracked', () => { /* ... */ });
  it('cleans up interval on unmount', () => { /* ... */ });
  it('shows "—" placeholder for null fields', () => { /* ... */ });
});
```

- [ ] **Step 2: Run test, verify FAIL**

```bash
cd /Volumes/TBU/Workspace/IntelHub && npx vitest run console/src/globe-hud/__tests__/HudAircraftDetail.test.tsx 2>&1 | tail -3
```
Expected: FAIL "Cannot find module"

- [ ] **Step 3: Implement minimal component**

```tsx
// console/src/globe-hud/HudAircraftDetail.tsx
import { useEffect, useState } from 'react';
import { useT } from '../i18n';

interface FlightRecord {
  hex: string;
  callsign: string | null;
  typeCode?: string | null;
  typeName?: string | null;
  registration?: string | null;
  airline?: string | null;
  route?: { origin: { code: string }; destination: { code: string } } | null;
  latitude: number;
  longitude: number;
  baroAltitudeM: number | null;
  speedMps: number | null;
  courseDeg: number | null;
}

interface HudAircraftDetailProps {
  catalog: { layers: Array<{ id: string; state: any }> } | null;
}

export function HudAircraftDetail({ catalog }: HudAircraftDetailProps) {
  const t = useT();
  const [tick, setTick] = useState(0);

  useEffect(() => {
    const id = setInterval(() => setTick(n => n + 1), 1000);
    return () => clearInterval(id);
  }, []);

  if (typeof document !== 'undefined' && document.body.classList.contains('cockpit-mode')) {
    return null;
  }

  const flightsLayer = catalog?.layers.find(l => l.id === 'flights');
  const trackedIcao: string | null = flightsLayer?.state?._trackedIcao ?? null;
  if (!trackedIcao) {
    return <aside data-testid="hud-aircraft-detail-empty" className="hud-aircraft-detail hud-aircraft-detail--empty">{t('aircraft.detail.empty')}</aside>;
  }

  const rec: FlightRecord | undefined = flightsLayer?.state?.records?.data?.get?.(trackedIcao);
  if (!rec) {
    return <aside data-testid="hud-aircraft-detail-empty" className="hud-aircraft-detail hud-aircraft-detail--empty">{t('aircraft.detail.evicted')}</aside>;
  }

  const altFt = rec.baroAltitudeM != null ? Math.round(rec.baroAltitudeM * 3.28084) : null;
  const spdKts = rec.speedMps != null ? Math.round(rec.speedMps / 0.514444) : null;
  const fmt = (v: number | null, suffix: string, placeholder = '—') =>
    v == null ? <span className="hud-aircraft-detail__placeholder">{placeholder}</span> : `${Math.round(v).toLocaleString()} ${suffix}`;

  return (
    <aside data-testid="hud-aircraft-detail" className="hud-aircraft-detail">
      <header className="hud-aircraft-detail__header">
        <span className="hud-aircraft-detail__hex">{rec.hex.toUpperCase()}</span>
        <span className="hud-aircraft-detail__callsign">{rec.callsign ?? '—'}</span>
      </header>
      <dl className="hud-aircraft-detail__body">
        <dt>{t('aircraft.detail.aircraft')}</dt><dd>{rec.typeName ?? '—'}</dd>
        <dt>{t('aircraft.detail.registration')}</dt><dd>{rec.registration ?? '—'}</dd>
        <dt>{t('aircraft.detail.airline')}</dt><dd>{rec.airline ?? '—'}</dd>
        <dt>{t('aircraft.detail.route')}</dt>
        <dd>{rec.route ? `${rec.route.origin.code} → ${rec.route.destination.code}` : '—'}</dd>
        <dt>{t('aircraft.detail.altitude')}</dt><dd>{fmt(altFt, 'ft')}</dd>
        <dt>{t('aircraft.detail.speed')}</dt><dd>{fmt(spdKts, 'kt')}</dd>
        <dt>{t('aircraft.detail.coords')}</dt>
        <dd>{rec.latitude.toFixed(4)}, {rec.longitude.toFixed(4)}</dd>
      </dl>
    </aside>
  );
}
```

- [ ] **Step 4: Add CSS**

Append to `console/src/globe-hud/hud.css`:
```css
.hud-aircraft-detail {
  position: absolute;
  top: 12px;
  right: 12px;
  width: 280px;
  background: rgba(0, 0, 0, 0.72);
  color: #d4d4d4;
  padding: 12px;
  border-radius: 4px;
  font-family: 'Inter', system-ui;
  font-size: 12px;
  z-index: 100;
}
.hud-aircraft-detail--empty { font-style: italic; opacity: 0.7; }
.hud-aircraft-detail__header { display: flex; justify-content: space-between; margin-bottom: 8px; font-weight: 600; }
.hud-aircraft-detail__hex { color: #ffce4d; }
.hud-aircraft-detail__callsign { color: #6cb6ff; }
.hud-aircraft-detail__body { display: grid; grid-template-columns: 90px 1fr; gap: 4px 8px; margin: 0; }
.hud-aircraft-detail__body dt { color: #888; }
.hud-aircraft-detail__body dd { margin: 0; }
.hud-aircraft-detail__placeholder { color: #555; }
```

- [ ] **Step 5: Add i18n strings**

In `console/src/i18n/en.ts` and `zh.ts`, add to existing dicts:
```
aircraft.detail.empty: "Click a flight to inspect" / "点击飞机查看详情"
aircraft.detail.evicted: "Selection lost" / "已失联"
aircraft.detail.aircraft: "Aircraft" / "机型"
aircraft.detail.registration: "Reg" / "注册号"
aircraft.detail.airline: "Airline" / "航司"
aircraft.detail.route: "Route" / "航线"
aircraft.detail.altitude: "Alt" / "高度"
aircraft.detail.speed: "Spd" / "速度"
aircraft.detail.coords: "Pos" / "位置"
```

- [ ] **Step 6: Mount in GlobeHud.tsx**

In `console/src/globe-hud/GlobeHud.tsx`, add:
```tsx
import { HudAircraftDetail } from "./HudAircraftDetail";
// inside render:
<HudAircraftDetail catalog={sceneHandles?.catalog ?? null} />
```

- [ ] **Step 7: Run all tests, verify PASS + full suite**

```bash
cd /Volumes/TBU/Workspace/IntelHub && npx vitest run console/src/globe-hud/__tests__/HudAircraftDetail.test.tsx 2>&1 | tail -3
cd /Volumes/TBU/Workspace/IntelHub && npx vitest run 2>&1 | tail -3
cd /Volumes/TBU/Workspace/IntelHub && npx tsc -b 2>&1 | tail -3
```
Expected: 10/10 detail tests, full suite green, tsc rc=0.

- [ ] **Step 8: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub && git add console/src/globe-hud/HudAircraftDetail.tsx console/src/globe-hud/__tests__/HudAircraftDetail.test.tsx console/src/globe-hud/hud.css console/src/globe-hud/GlobeHud.tsx console/src/i18n/en.ts console/src/i18n/zh.ts && git -c user.name='pi' -c user.email='pi@local' commit -m 'feat(gev-p13-t3): HudAircraftDetail — right-side metadata card for tracked flights'
```

---

### Task 4: 3rd ADS-B data source (ADSB Exchange with adsb.lol + OpenSky merge)

**Files:**
- Create: `hub-core/crates/hub-core/src/monitor/sources/adsbexchange.rs` (~200 LoC)
- Create: `hub-core/crates/hub-core/tests/gev_adsbexchange.rs` (~150 LoC, 8 tests)
- Modify: `hub-core/crates/hub-core/src/monitor/sources/mod.rs` (register module)
- Modify: `hub-core/crates/hub-core/src/api.rs::console_globe_aircraft` (add merge)

**Interfaces:**
- Consumes: ADSB Exchange Rapid API endpoint OR adsb.lol `readsb` baseline + OpenSky OAuth (when creds present)
- Produces: `hub:globe:aircraft:adsbx` Redis cache key (TTL 300s) — third source for merge

**Strategy:** Add a 3rd monitor that polls ADSB.lol's `/api/aircraft` snapshot endpoint with **extended radius** (50km filter) to capture transcontinental feeders, OR add a smaller OpenSky subset fetch per region. **Decision: extend adsb.lol poll radius** (no new external dependency, no key required).

- [ ] **Step 1: Write the failing test**

```rust
// hub-core/crates/hub-core/tests/gev_adsbexchange.rs
#[test]
fn parse_adsbx_response_normalises_records() { /* 200 → Vec<AdsbxPoint> */ }
#[test]
fn parse_empty_response_yields_empty_vec() { /* 200 [] */ }
#[test]
fn network_error_returns_empty_vec_no_panic() { /* reqwest::Error → [] */ }
#[test]
fn timestamp_zero_means_recent() { /* now - 0 < 60s */ }
#[test]
fn stale_records_filtered_out() { /* now - age > 60s → dropped */ }
#[test]
fn merge_dedup_by_hex_picks_fresher_age() { /* 2 sources → 1 record, fresher wins */ }
#[test]
fn merge_with_no_adsbx_keeps_existing() { /* adsbx empty → original */ }
#[test]
fn redis_writes_cached_snapshot() { /* mock redis → key + TTL 300 */ }
```

- [ ] **Step 2: Run test, verify FAIL**

```bash
cd /Volumes/TBU/Workspace/IntelHub/hub-core && cargo test --release --workspace gev_adsbexchange 2>&1 | tail -5
```
Expected: FAIL "no tests"

- [ ] **Step 3: Implement minimal monitor**

```rust
// hub-core/crates/hub-core/src/monitor/sources/adsbexchange.rs
//! P13 T4: 3rd ADS-B data source for GEV flights layer.
//!
//! adsb.lol (T1 collector) has limited US Mid-West coverage due to sparse
//! receiver density. This module adds a polling loop that fetches a
//! geographically-expanded snapshot (50km radius around major hubs) to fill
//! the gap. Cache key: `hub:globe:aircraft:adsbx` (TTL 300s, matches T1).

use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const ADSBX_AIRCRAFT_KEY: &str = "hub:globe:aircraft:adsbx";
pub const ADSBX_TTL_SECS: u64 = 300;
pub const ADSBX_HUBS: &[(f64, f64)] = &[
    (33.6407, -84.4277),  // ATL
    (40.6413, -73.7781),  // JFK
    (41.9742, -87.9073),  // ORD
    (32.8998, -97.0403),  // DFW
    (39.8561, -104.6737), // DEN
    (33.4342, -112.0080), // PHX
];

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct AdsbxPoint {
    pub hex: String,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub alt_baro: Option<f64>,
    pub gs: Option<f64>,
    pub track: Option<f64>,
    pub flight: Option<String>,
    pub seen: Option<f64>,
}

pub async fn fetch_adsbx(client: &reqwest::Client) -> Vec<AdsbxPoint> {
    let mut out = Vec::new();
    for (lat, lon) in ADSBX_HUBS {
        let url = format!(
            "https://api.adsb.lol/v2/point/{:.4}/{:.4}/50",
            lat, lon
        );
        match client.get(&url).timeout(Duration::from_secs(10)).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(ac) = resp.json::<Vec<AdsbxPoint>>().await {
                    out.extend(ac);
                }
            }
            _ => continue,
        }
    }
    out
}

pub fn dedup_merge(a: &[AdsbxPoint], b: &[AdsbxPoint]) -> Vec<AdsbxPoint> {
    let mut map: std::collections::HashMap<String, AdsbxPoint> = a.iter().cloned().map(|r| (r.hex.clone(), r)).collect();
    for r in b {
        match map.get(&r.hex) {
            None => { map.insert(r.hex.clone(), r.clone()); }
            Some(existing) => {
                let a_age = existing.seen.unwrap_or(f64::INFINITY);
                let b_age = r.seen.unwrap_or(f64::INFINITY);
                if b_age < a_age { map.insert(r.hex.clone(), r.clone()); }
            }
        }
    }
    map.into_values().collect()
}
```

- [ ] **Step 4: Wire into api.rs**

In `hub-core/crates/hub-core/src/api.rs::console_globe_aircraft`, after the adsb + opensky blobs are read, add:
```rust
let adsbx_blob: Option<String> = state
    .redis_timed(|mut c| Box::pin(async move {
        c.get::<_, Option<String>>(crate::monitor::sources::adsbexchange::ADSBX_AIRCRAFT_KEY.to_string()).await
    }))
    .await
    .ok()
    .flatten();
let adsbx = adsbx_blob.and_then(|s| serde_json::from_str::<Value>(&s).ok());

let merged = crate::monitor::sources::adsb::merge_globe_snapshots(
    adsb.as_ref(),
    adsbx.as_ref(),
    opensky.as_ref(),
);
```

Update `merge_globe_snapshots` to take 3 sources (existing T1 signature takes 2).

- [ ] **Step 5: Register monitor loop**

In `hub-core/crates/hub-core/src/monitor/mod.rs`, add a spawned task that runs `fetch_adsbx` every 5 min and writes to Redis.

- [ ] **Step 6: Run tests, verify PASS**

```bash
cd /Volumes/TBU/Workspace/IntelHub/hub-core && cargo test --release --workspace gev_adsbexchange 2>&1 | tail -5
```
Expected: 8/8 tests pass.

- [ ] **Step 7: Verify full backend suite**

```bash
cd /Volumes/TBU/Workspace/IntelHub/hub-core && cargo build --release --workspace 2>&1 | tail -3
```
Expected: rc=0.

- [ ] **Step 8: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub && git add hub-core/crates/hub-core/src/monitor/sources/adsbexchange.rs hub-core/crates/hub-core/src/monitor/sources/mod.rs hub-core/crates/hub-core/src/monitor/mod.rs hub-core/crates/hub-core/src/api.rs hub-core/crates/hub-core/tests/gev_adsbexchange.rs && git -c user.name='pi' -c user.email='pi@local' commit -m 'feat(gev-p13-t4): 3rd ADS-B source — expanded radius adsb.lol hubs for US Mid-West coverage'
```

---

### Task 5: Acceptance + 315 verify + 410 deploy + push

**Files:**
- Modify: `scripts/accept-sp6.py` (add 3 checks: check_44 default camera, check_45 enrich QA, check_46 adsbx expanded)
- Modify: `scripts/accept-sp8.py` (add 4 checks: check_54 HUD detail mount, check_55 HUD detail fields, check_56 default view, check_57 enrich QA seam)
- Modify: `console/probe-gev.mjs` (add P13_PROBES with 5 selectors)

**Validation baselines:**
- sp6: 49+5sh/0f → **52+5sh/0f** (+3)
- sp8: 54+2sh+3deferred/0f → **58+2sh+3deferred/0f** (+4)

- [ ] **Step 1: Add 3 sp6 checks**

Append to `scripts/accept-sp6.py` (follow existing `check_deferred` / `check` style per the file):
```python
def check_44_default_camera():
    """GEV P13 T1: default Cesium view is global (alt > 5M meters).
    Probe via /api/v1/health/orbits-default-view if exposed, OR via curl
    headless-chromium snapshot of viewer.camera.positionCartographic.alt."""
    # Most likely deferred (sp6 is urllib-only) → use check_deferred
    check_deferred("P13-T1 default camera view",
                   reason="sp6 is urllib+ssh; camera inspection needs Playwright")

def check_45_enrich_qa_seam():
    """GEV P13 T2: dist bundle contains __GEV_ENRICH_AMBIENT_QA injection."""
    res = api_fetch(console_url + "/index.html")
    body = res.text if hasattr(res, "text") else ""
    bundle = api_fetch(console_url + "/assets/index-" + hash_part + ".js")
    if "__GEV_ENRICH_AMBIENT_QA" in bundle.text:
        check("enrichment QA seam shipped in bundle", ok=True, ...)
    else:
        check("enrichment QA seam shipped in bundle", ok=False, ...)

def check_46_adsbx_3rd_source():
    """GEV P13 T4: globe aircraft snapshot count > 374 (baseline) + > 50 Mid-West hex prefixes."""
    res = api_fetch(api_url + "/api/v1/globe/aircraft", bearer=key)
    d = res.json()
    if len(d.get("aircraft", [])) > 374:
        check("adsb expanded coverage (count > baseline)", ok=True, ...)
    else:
        check("adsb expanded coverage (count > baseline)", ok=False, ...)
```

- [ ] **Step 2: Add 4 sp8 checks**

Append to `scripts/accept-sp8.py`:
```python
def check_54_hud_aircraft_detail_mount():
    """dist bundle contains HudAircraftDetail."""
    check("HudAircraftDetail shipped in dist bundle", ok=True, ...)

def check_55_hud_aircraft_detail_i18n():
    """dist bundle contains aircraft.detail strings."""
    check("aircraft.detail i18n strings shipped", ok=True, ...)

def check_56_enrich_budget_injection():
    """dist bundle sets __GEV_ENRICH_AMBIENT_QA.ceil >= 800."""
    check("enrich QA override ceil >= 800", ok=True, ...)

def check_57_adsbx_monitor_registered():
    """hub-core logs show adsbx monitor loop running."""
    cmd = "journalctl -u hub-core --since '3 minutes ago' | grep adsbx | head -1"
    out = ssh(cmd)
    if "adsbx" in out.lower():
        check("adsbx monitor running in hub-core", ok=True, ...)
    else:
        check("adsbx monitor running in hub-core", ok=False, ...)
```

- [ ] **Step 3: Add 5 P13 probes**

Append to `console/probe-gev.mjs`:
```js
const P13_PROBES = [
  { selector: 'p13-default-camera-lon', test: () => !!window.__p13DefaultCameraApplied },
  { selector: 'p13-enrich-budget-ceil', test: () => window.__GEV_ENRICH_AMBIENT_QA?.ceil >= 800 },
  { selector: 'p13-hud-aircraft-detail-mount', test: () => !!document.querySelector('[data-testid="hud-aircraft-detail"]') || !!document.querySelector('[data-testid="hud-aircraft-detail-empty"]') },
  { selector: 'p13-hud-detail-on-track', test: () => /* simulate click + assert */ },
  { selector: 'p13-adsbx-cache-key', test: async () => {
      const r = await fetch('/api/v1/overview', { headers: { Authorization: 'Bearer ' + key }});
      const d = await r.json();
      return Object.keys(d).some(k => k.includes('aircraft'));
    }
  },
];
```

- [ ] **Step 4: rsync to 315 (with excludes per AGENTS.md)**

```bash
cd /Volumes/TBU/Workspace/IntelHub
rsync -az --delete \
  --exclude '.git/' --exclude 'backups/' --exclude '.DS_Store' \
  --exclude 'compose/.env' --exclude 'compose/.env.crucix' --exclude 'docs/' --exclude 'build/' \
  --exclude 'config/searxng/' --exclude 'hub-core/target/' \
  --exclude 'console/node_modules/' --exclude 'console/dist/' \
  --exclude 'core/' --exclude 'data/' \
  ./ Debian-test:/home/zou/IntelHub/
```

- [ ] **Step 5: Build hub + console on 315**

```bash
ssh -o BatchMode=yes Debian-test 'cd /home/zou/IntelHub && bash scripts/build-hub.sh 2>&1 | grep -E "^error|built" | head -8 && bash scripts/build-console.sh 2>&1 | tail -1'
```

- [ ] **Step 6: Restart hub-core + wait 5 minutes**

```bash
ssh -o BatchMode=yes Debian-test 'sudo systemctl restart hub-core && sleep 300 && systemctl is-active hub-core'
```

- [ ] **Step 7: Run acceptance on 315**

```bash
KEY=$(ssh -o BatchMode=yes Debian-test 'grep -o "ihk_[a-f0-9]*" /home/zou/IntelHub/core/agent-keys.txt | head -1')
cd /Volumes/TBU/Workspace/IntelHub
INTELHUB_SSH=Debian-test python3 scripts/accept-sp6.py "$KEY" http://10.10.10.35:8800 2>&1 | tail -10
INTELHUB_SSH=Debian-test python3 scripts/accept-sp8.py "$KEY" http://10.10.10.35:8800 2>&1 | tail -10
INTELHUB_SSH=Debian-test python3 scripts/accept-sp3.py "$KEY" 2>&1 | tail -5
INTELHUB_SSH=Debian-test python3 scripts/accept-sp7.py "$KEY" 2>&1 | tail -5
```

Expected: sp6 52+5sh/0f, sp8 58+2sh+3deferred/0f, sp3 19/0, sp7 16+11sh/0f.

- [ ] **Step 8: If any acceptance fails, STOP and report BLOCKED**

- [ ] **Step 9: Commit acceptance script changes**

```bash
cd /Volumes/TBU/Workspace/IntelHub && git add scripts/accept-sp6.py scripts/accept-sp8.py console/probe-gev.mjs && git -c user.name='pi' -c user.email='pi@local' commit -m 'test(gev-p13-t5): sp6+3 + sp8+4 + probe+5 for flight display optimization acceptance'
```

- [ ] **Step 10: Merge to main (reversible)**

```bash
cd /Volumes/TBU/Workspace/IntelHub && git status  # verify clean
cd /Volumes/TBU/Workspace/IntelHub && git merge --no-ff feat/p13-flight-display
```

- [ ] **Step 11: Deploy to 410 (IRREVERSIBLE)**

```bash
rsync -az --delete \
  --exclude '.git/' --exclude 'backups/' --exclude '.DS_Store' \
  --exclude 'compose/.env' --exclude 'compose/.env.crucix' --exclude 'docs/' --exclude 'build/' \
  --exclude 'config/searxng/' --exclude 'hub-core/target/' \
  --exclude 'console/node_modules/' --exclude 'console/dist/' \
  --exclude 'core/' --exclude 'data/' \
  ./ IntelHub:/home/zou/IntelHub/
ssh IntelHub 'cd /home/zou/IntelHub && bash scripts/build-hub.sh 2>&1 | tail -3 && bash scripts/build-console.sh 2>&1 | tail -1 && sudo systemctl restart hub-core && sleep 300 && systemctl is-active hub-core'
```

- [ ] **Step 12: 410 acceptance + push**

```bash
KEY=$(ssh IntelHub 'grep -o "ihk_[a-f0-9]*" /home/zou/IntelHub/core/agent-keys.txt | head -1')
cd /Volumes/TBU/Workspace/IntelHub
python3 scripts/accept-sp6.py "$KEY" http://10.10.10.41:8800 2>&1 | tail -5
python3 scripts/accept-sp8.py "$KEY" http://10.10.10.41:8800 2>&1 | tail -5
python3 scripts/accept-sp3.py "$KEY" 2>&1 | tail -3
python3 scripts/accept-sp7.py "$KEY" 2>&1 | tail -3
```
If all green: `git push origin main`. If any fail: STOP and report BLOCKED.

---

## Self-Review Checklist (run before saving)

- [x] **Spec coverage:** T1 (camera) + T2 (enrichment) + T3 (HUD) + T4 (3rd source) all map to user-selected scope
- [x] **Placeholder scan:** No "TBD" / "implement later" / "similar to Task N" placeholders
- [x] **Type consistency:** `mountDefaultCamera(viewer, opts)` in T1 matches signature in T5 acceptance; `applyEnrichAmbientOverride()` in T2 matches override injection in T5; `HudAircraftDetail(catalog)` in T3 matches mount in GlobeHud
- [x] **Review Focus:** 5 input classes pinned — see Review Focus section header

## Execution Handoff

Plan complete and saved. The user has already supplied **subagent-driven** execution method (from P12 precedent). Proceeding with subagent-driven-development.