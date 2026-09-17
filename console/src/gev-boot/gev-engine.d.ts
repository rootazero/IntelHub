// Type channel for the vendored GEV engine (console/gev-engine/, plain JS).
// Per controller ruling 1: NO @ts-expect-error at import sites in later tasks
// (T7 bootstrap etc.) — this declaration file is the single typing path.
//
// VERIFIED FORM: single-star wildcard + shorthand (no body). Properties:
//   - `declare module "gev-engine/*";` is LEGAL ambient syntax; the double-star
//     `*gev-engine*` pattern from the first attempt was NOT (TS5061 "pattern
//     contains a wildcard character of invalid form") — and tsconfig has
//     skipLibCheck: true, which SILENTLY SWALLOWS TS5061, so the module was
//     never registered and the first named engine import would have failed
//     TS7016. Never reintroduce a double-star pattern here.
//   - The shorthand form (no `{ ... }` body) types every export as `any`,
//     unlike `export = anyExport` which rejects named imports (TS2305).
//   - Depth-independent: one line covers every `gev-engine/<any depth>` module.
//
// Import side (verified end-to-end via tsc --noEmit probe, see task-2 report):
//   console TS code imports bare specifiers like
//   `import { createApplication } from "gev-engine/src/app/application.js"`.
//   tsc resolves the specifier via tsconfig paths ("gev-engine/*") and this
//   ambient wildcard; vite/vitest resolve it via resolve.alias in
//   vite.config.ts / vitest.config.ts. All three must stay in sync.
declare module "gev-engine/*";
