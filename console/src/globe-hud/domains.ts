// HUD layer domains (T9): the IntelHub-owned grouping of the engine's
// LAYER_STATE_REGISTRY layer ids for the left-rail flyout UI.
//
// Grouping-source decision (verified against vendor, see task-9 report):
// the canonical registry (gev-engine/src/data/layerState.js) carries only
// {id, token, disposition} — NO group/label field — and the layer instances
// built by app/constructCatalog.js carry name/icon/source but no group
// either. So the domain table is an IntelHub HUD concern, keyed by the
// canonical registry ids (NOT the IntelHub adapter source names — e.g. the
// registry id is "ais-live-vessels", not the "vessels" source key).
//
// Invariants enforced by __tests__/hud-layer-rail.test.tsx:
//   - every LAYER_STATE_REGISTRY id appears in EXACTLY ONE domain
//   - every id listed here exists in LAYER_STATE_REGISTRY (no drift)
//
// The 7 domains follow the HUD design vote (✈🛰🏙🚢🌐🏭🔥). cyber and infra
// were separate draft rows that both claimed "installations"; with the real
// registry ids they split cleanly: cyber takes the datacenter layer, infra
// takes military installations + dams.
export interface HudDomain {
  id: string;
  icon: string;
  label: { zh: string; en: string };
  /** Canonical LAYER_STATE_REGISTRY layer ids in this domain. */
  layers: readonly string[];
}

export const DOMAINS: readonly HudDomain[] = [
  {
    id: "air",
    icon: "✈",
    label: { zh: "航空", en: "Air" },
    layers: ["flights", "military", "military-awareness"],
  },
  {
    id: "space",
    icon: "🛰",
    label: { zh: "太空", en: "Space" },
    layers: ["satellites", "rocket-launches"],
  },
  {
    id: "ground",
    icon: "🏙",
    label: { zh: "地面", en: "Ground" },
    layers: [
      "traffic",
      "transit",
      "directions",
      "cctv",
      "alpr-cameras",
      "bikeshare",
      "radio",
    ],
  },
  {
    id: "sea",
    icon: "🚢",
    label: { zh: "海洋", en: "Sea" },
    layers: ["ais-live-vessels", "telegeography-submarine-cables"],
  },
  {
    id: "cyber",
    icon: "🌐",
    label: { zh: "网络", en: "Cyber" },
    layers: ["local-datacenters"],
  },
  {
    id: "infra",
    icon: "🏭",
    label: { zh: "基建", en: "Infra" },
    layers: ["military-installations", "local-dams"],
  },
  {
    id: "env",
    icon: "🔥",
    label: { zh: "环境", en: "Environment" },
    layers: [
      "local-firms",
      "earthquakes",
      "bhote-koshi-2026",
      "bhote-koshi-locator",
    ],
  },
];

/** layerId → domain, for flyout lookup. Derived from DOMAINS at module load. */
export const DOMAIN_BY_LAYER_ID: Readonly<Record<string, HudDomain>> =
  Object.freeze(
    Object.fromEntries(
      DOMAINS.flatMap((domain) =>
        domain.layers.map((layerId) => [layerId, domain]),
      ),
    ),
  );
