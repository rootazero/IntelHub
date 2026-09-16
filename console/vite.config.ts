import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import cesium from "vite-plugin-cesium";

export default defineConfig({
  // rebuildCesium: bundle cesium into the lazy Globe chunk instead of
  // injecting a 6.1MB global Cesium.js script into every route's index.html.
  // (The plugin still injects the /cesium/Widgets/widgets.css link — do NOT
  // import widgets.css from Globe.tsx or it loads twice.)
  plugins: [react(), tailwindcss(), cesium({ rebuildCesium: true })],
  build: { outDir: "dist", sourcemap: false },
});
