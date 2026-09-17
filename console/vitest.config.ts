import { fileURLToPath, URL } from "node:url";
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  resolve: {
    // MUST mirror vite.config.ts: when this file exists vitest ignores
    // vite.config.ts entirely, so the engine alias (and the define keys
    // below) have to be repeated here or tests run keyless/broken.
    alias: {
      "gev-engine": fileURLToPath(new URL("./gev-engine", import.meta.url)),
    },
  },
  define: {
    "import.meta.env.GOOGLE_MAPS_API_KEY": JSON.stringify(process.env.VITE_GOOGLE_MAPS_KEY ?? ""),
    "import.meta.env.CESIUM_ION_TOKEN": JSON.stringify(process.env.VITE_ION_KEY ?? ""),
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test-setup.ts"],
    // Scope to console's own tests: the vendored engine ships hundreds of
    // upstream *.test.mjs files (run by upstream's own tooling), which must
    // not be picked up by this vitest project.
    include: ["src/**/*.test.{ts,tsx}", "src/**/*.spec.{ts,tsx}"],
  },
});
