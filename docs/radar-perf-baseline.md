# Radar perf baseline (pre-fix)

Captured via `console/probe-radar-marker-perf.mjs` against prod
(10.10.10.41, build from 2026-09-13) on 2026-09-18.

```
{
  "domMarkers": 1215,
  "wrap": "radar-rel",
  "samples": 807,
  "p50": 16.6,
  "p95": 17.5,
  "p99": 17.9,
  "max": 22.4,
  "pageErrors": []
}
```

**Note**: Frame deltas in headless chromium do not reflect real-world
GPU compositing cost — the user's symptom (visible lag when sliding
the mouse over markers) is a real-browser GPU paint/composite issue.
Headless software-rendered WebGL hides the cost. The reliable signal
is **domMarkers**: 1215 DOM `.radar-marker` divs participate in
hit-testing on every mousemove.

Post-fix target: `domMarkers === 0` (markers rendered to the
MapLibre canvas via a circle layer, no per-event DOM).
