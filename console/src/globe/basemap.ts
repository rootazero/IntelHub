// Globe basemap chain — single source of truth for the /globe Cesium page.
// Ordered best → worst, keyless esri always reachable (spec §4.2):
//   - VITE_GOOGLE_MAPS_KEY → Google Photorealistic 3D Tiles
//   - VITE_CESIUM_ION_KEY  → Cesium World Terrain + Cesium World Imagery
//   - (none)               → EllipsoidTerrain + Esri World Imagery (keyless)
// Keys are build-time injected via core/console-build.env on the VM
// (same chain as VITE_CARTO_KEY); never committed.

import * as Cesium from "cesium";

export const ION_KEY = (import.meta.env.VITE_CESIUM_ION_KEY as string | undefined) ?? "";
export const GOOGLE_KEY = (import.meta.env.VITE_GOOGLE_MAPS_KEY as string | undefined) ?? "";

export type GlobeBase = "google" | "ion" | "esri";

export function globeBase(): GlobeBase {
  if (GOOGLE_KEY) return "google";
  if (ION_KEY) return "ion";
  return "esri";
}

/** Apply the best available imagery+terrain. Never throws — esri is keyless. */
export async function applyBasemap(viewer: Cesium.Viewer): Promise<GlobeBase> {
  const base = globeBase();
  try {
    if (base === "google") {
      viewer.imageryLayers.removeAll();
      const tileset = await Cesium.createGooglePhotorealistic3DTileset({ key: GOOGLE_KEY });
      viewer.scene.primitives.add(tileset);
      return "google";
    }
    if (base === "ion") {
      Cesium.Ion.defaultAccessToken = ION_KEY;
      viewer.imageryLayers.removeAll();
      viewer.imageryLayers.addImageryProvider(await Cesium.IonImageryProvider.fromAssetId(3954));
      viewer.terrainProvider = await Cesium.CesiumTerrainProvider.fromIonAssetId(1);
      return "ion";
    }
  } catch (e) {
    console.warn("[globe] basemap upgrade failed, falling back to esri", e);
  }
  viewer.imageryLayers.removeAll();
  viewer.imageryLayers.addImageryProvider(
    new Cesium.UrlTemplateImageryProvider({
      url: "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}",
      credit: "Esri, Maxar, Earthstar Geographics",
    }),
  );
  viewer.terrainProvider = new Cesium.EllipsoidTerrainProvider();
  return "esri";
}
