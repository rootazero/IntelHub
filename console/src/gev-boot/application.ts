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
//   * Engine createApplicationTools is NOT used: it destructures
//     controls.styleManager and data.dataManager (tools.js:20-23) and installs
//     the scope mask + engine chrome — IntelHub's HUD owns that chrome. It
//     stays stubbed at `{}`.
//
// Ruling 9 (T8): the DATA phase IS wired, but with IntelHub's own createData
// following the vendor recipe printed in gev-engine/src/app/data.js
// (verified against src/data/lifecycle.js):
//   new LayerLifecycle(viewer, { allowQaRegistration: false })
//   → per-layer register() → attachDataManager?.() → attachMapStackController?.()
//   → finalizeRegistrations(catalog.metadata) → defer(destroyAll())
// Engine createApplicationData itself is NOT called: it additionally mounts
// the engine's LayerPresentation into #data-toggles and requires
// controls.styleManager, both engine chrome the IntelHub HUD replaces (T9).
// T9's layer rail drives enable/disable via getComponents().data.dataManager.

import { createApplication } from "gev-engine/src/app/application.js";
import { createApplicationScene } from "gev-engine/src/app/scene.js";
import { createApplicationCatalog } from "gev-engine/src/app/constructCatalog.js";
import { LayerLifecycle } from "gev-engine/src/data/lifecycle.js";
import { setScopeMaskEnabled } from "gev-engine/src/scopeMask.js";
import { createIntelHubRequestServices } from "./request-services";
import { createIntelHubLayerSources } from "../gev-adapters";

// Default-enabled layer set (controller ruling 9). The vendor data.js has NO
// default-enable logic of its own (lifecycle entries start disabled; upstream
// enables come from state restoration, which IntelHub does not run), so the
// initial set is an IntelHub policy decision: the four wave-1 live layers on,
// everything else off.
const DEFAULT_ENABLED_LAYERS = [
  "flights",
  "military",
  "satellites",
  "earthquakes",
] as const;

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
    // Data phase (Ruling 9): register the catalog into a LayerLifecycle data
    // manager following the vendor data.js recipe. createData is sync — the
    // engine awaits the factory's return like any other phase.
    createData: ({ scene, controls, signal, defer }: any) => {
      void signal;
      const { viewer, mapStackController } = scene;
      const { catalog } = controls;
      // Vendor data.js:27 — the catalog is a hard requirement of this phase.
      if (!catalog?.layers || !catalog?.metadata)
        throw new TypeError("An application layer catalog is required");
      const dataManager = new LayerLifecycle(viewer, {
        allowQaRegistration: false,
      });
      defer(async () => {
        await dataManager.destroyAll();
        if (dataManager.layers.size)
          throw new Error(
            `Data layers could not be destroyed: ${[...dataManager.layers.keys()].join(", ")}`,
          );
      });
      for (const layer of catalog.layers) dataManager.register(layer);
      for (const layer of catalog.layers) layer.attachDataManager?.(dataManager);
      for (const layer of catalog.layers)
        layer.attachMapStackController?.(mapStackController);
      // Seals registration; catalog.metadata is the LAYER_STATE_REGISTRY array
      // of {id, token, disposition} and must cover every registered layer.
      dataManager.finalizeRegistrations(catalog.metadata);
      // Fire-and-forget: enabling runs each layer's init/enable/first-update
      // (which hits live sources via apiFetch) on the per-layer queue —
      // blocking start() on first data would delay the page for no UX gain.
      for (const id of DEFAULT_ENABLED_LAYERS) {
        void Promise.resolve(
          dataManager.setEnabled(id, true, { origin: "programmatic" }),
        ).catch((e) => console.warn(`[gev] enable ${id} failed:`, e));
      }
      // T9's rail reaches the manager via getComponents().data.dataManager.
      return { dataManager, lifecycle: dataManager };
    },
    // Engine tools phase is deliberately stubbed (file header) — IntelHub's
    // HUD owns its own layer UI instead of the engine chrome installed here.
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
