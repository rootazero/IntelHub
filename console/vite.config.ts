import { fileURLToPath, URL } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import cesium from "vite-plugin-cesium";

// The vendored engine's curated local_data GeoJSON packs (multi-MB DataSF /
// Natural Earth datasets) were not vendored with the source. Three dynamic
// import()s reference them (neighborhoods/san-francisco.json, natural_earth/
// regions.json, natural_earth/marine.json). Both consumers handle a failed
// load gracefully by design (neighborhoodPolygons falls back to the live
// resolver ladder; naturalEarthRegions callers `.catch(() => null)`), so the
// imports are left external and the runtime fetch 404s into those documented
// fallback paths instead of failing the production build.
const gevLocalDataExternal = {
  name: "gev-local-data-external",
  resolveId(source: string) {
    return /local_data\/[^/]+\/[^/]+\.json$/.test(source)
      ? { id: source, external: true }
      : null;
  },
};

export default defineConfig({
  // rebuildCesium: bundle cesium into the lazy Globe chunk instead of
  // injecting a 6.1MB global Cesium.js script into every route's index.html.
  // (The plugin still injects the /cesium/Widgets/widgets.css link — do NOT
  // import widgets.css from Globe.tsx or it loads twice.)
  plugins: [react(), tailwindcss(), cesium({ rebuildCesium: true }), gevLocalDataExternal],
  resolve: {
    // Bare-specifier import channel for the vendored GEV engine: console TS
    // code imports `gev-engine/src/app/application.js` etc. instead of fragile
    // depth-relative paths. Vite resolves at bundle time; tsc resolves via
    // tsconfig paths + the ambient wildcard in src/gev-boot/gev-engine.d.ts.
    alias: {
      "gev-engine": fileURLToPath(new URL("./gev-engine", import.meta.url)),
    },
  },
  define: {
    // Vendored GEV engine reads these at module scope (upstream build/vite.js
    // equivalent). Fold build-time keys in so `import.meta.env.X` resolves in
    // the engine's plain-JS modules. build-console.sh passes VITE_GOOGLE_MAPS_KEY
    // / VITE_ION_KEY through docker; empty fallback keeps the bundle buildable.
    "import.meta.env.GOOGLE_MAPS_API_KEY": JSON.stringify(process.env.VITE_GOOGLE_MAPS_KEY ?? ""),
    "import.meta.env.CESIUM_ION_TOKEN": JSON.stringify(process.env.VITE_ION_KEY ?? ""),
  },
  build: { outDir: "dist", sourcemap: false },
});
