// Bootstrap core: wire the vendored GEV engine's generic four-phase lifecycle
// (app/application.js) to IntelHub-owned sources and services.
//
// This is the ONLY place the engine's createApplication factories are
// assembled; the HUD page (T8) interacts solely with the
// `createIntelHubGlobe` signature below. Engine imports use the bare
// `gev-engine/...` alias (controller ruling 1+6): no relative paths and no
// ts-expect-error suppression comments — typing comes from
// src/gev-boot/gev-engine.d.ts.
//
// Step-1 phase-attribution findings (gev-engine/src/app/):
//   * catalog needs `scene.operations.surface` (constructCatalog.js:67-69),
//     so it is built in the CONTROLS phase, right after scene — matching the
//     brief. The engine's own createApplicationData has a hard catalog
//     dependency (data.js:27), but the IntelHub bootstrap does not call the
//     engine's data/tools factories at all (see below), so no phase change.
//   * Engine createApplicationData is NOT used: it mounts a #data-toggles
//     widget, requires controls.styleManager, and hard-requires the catalog
//     (data.js:27). Engine createApplicationTools is NOT used either: it
//     destructures controls.styleManager and data.dataManager (tools.js:20-23)
//     and installs the scope mask + engine chrome. Both stay stubbed at `{}`;
//     T9 wires IntelHub's own data/layer sidebar against
//     getComponents().scene.catalog, and layers are constructed but not yet
//     registered into a LayerLifecycle data manager.

import { createApplication } from "gev-engine/src/app/application.js";
import { createApplicationScene } from "gev-engine/src/app/scene.js";
import { createApplicationCatalog } from "gev-engine/src/app/constructCatalog.js";
import { setScopeMaskEnabled } from "gev-engine/src/scopeMask.js";
import { createIntelHubRequestServices } from "./request-services";
import { createIntelHubLayerSources } from "../gev-adapters";

export interface IntelHubGlobeOptions {
  /** Authenticated hub transport, shared by adapters + future P4/P5 services. */
  apiFetch: Parameters<typeof createIntelHubLayerSources>[0]["apiFetch"];
  googleApiKey: string;
  cesiumToken: string;
}

/**
 * Create the IntelHub globe application. Does no DOM work until start() —
 * the factories run inside the engine's ordered scene→controls→data→tools
 * startup.
 */
export function createIntelHubGlobe(opts: IntelHubGlobeOptions) {
  const app = createApplication({
    createScene: ({ signal, defer }: any) =>
      createApplicationScene({
        requestServices: createIntelHubRequestServices(opts.apiFetch),
        googleApiKey: opts.googleApiKey,
        cesiumToken: opts.cesiumToken,
        // scene.js writes loaderStatus.textContent unconditionally — fall back
        // to a plain object when the HUD page has no #loading-screen element.
        loaderStatus:
          document.querySelector("#loading-screen .loader-status") ??
          ({ textContent: "" } as HTMLElement),
        signal,
        defer,
      }),
    createControls: ({ scene, signal, defer }: any) => {
      const sources = createIntelHubLayerSources({ apiFetch: opts.apiFetch });
      const catalog = createApplicationCatalog({
        surface: scene.operations.surface,
        sources,
        signal,
      });
      // T9-T11 reach the catalog via getComponents().scene.catalog.
      scene.catalog = catalog;
      return { catalog };
    },
    // Engine data/tools phases are deliberately stubbed (file header) — the
    // IntelHub HUD owns its own layer UI instead of #data-toggles.
    createData: async () => ({}),
    createTools: async () => ({}),
  });
  return {
    start: async () => {
      await app.start();
      // The scope mask (circular viewport treatment) is engine-chrome we do
      // not install — but disable defensively in case any path installs it.
      setScopeMaskEnabled(false);
    },
    destroy: async () => {
      await app.destroy();
    },
    getComponents: () => app.getComponents(),
  };
}
