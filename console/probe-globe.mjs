#!/usr/bin/env node
// probe-globe.mjs — RETIRED P1 probe (self-built 2D globe page).
//
// The /globe route now mounts the GEV-engine page (GlobeV2, T8), so the P1
// assertions ([data-probe="aircraft-count"], a bare canvas lookup) no longer
// describe anything on that route — left as-is they would report a green
// "probe-globe OK" while checking nothing. The v2 probe lives in
// ./probe-gev.mjs (T14); this file stays a pass-through so the old invocation
// path (T17/T18, INTELHUB_SSH=Debian-test node console/probe-globe.mjs …) runs
// the real assertions until T16 deletes it.
//
// Usage/output are exactly console/probe-gev.mjs — see its header.
import "./probe-gev.mjs";
