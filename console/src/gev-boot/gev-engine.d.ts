// Type channel for the vendored GEV engine (console/gev-engine/, plain JS).
// Per controller ruling 1: NO @ts-expect-error at import sites in later tasks
// (T7 bootstrap etc.) — this declaration file is the single typing path.
//
// Wildcard ambient module: matches any specifier containing "gev-engine",
// including deep relative imports like "../../gev-engine/src/app/application.js".
declare module "*gev-engine*" {
  const anyExport: any;
  export = anyExport;
}
