// kindmeta tests — L1 (primary display) taxonomy + L2 → L1 mapping.
//
// §FE-RADAR-PRIMARY-CATEGORIES (2026-09-28):
// Radar chips / dropdown / legend surface a 15-category L1 taxonomy
// for human viewing, while the underlying L2 kinds remain unchanged
// in the data layer (Signal.kind, geo_events.kind, agent/MCP queries).
//
// The map below pins the exact L2 → L1 routing. Any source that emits
// a new L2 without updating KIND_TO_PRIMARY falls through to `other`
// (defensive default — see primaryOf()).

import { describe, it, expect } from "vitest";
import {
  PRIMARY_KINDS,
  KIND_TO_PRIMARY,
  PRIMARY_COLOR,
  primaryOf,
} from "../kindmeta";

describe("PRIMARY_KINDS — L1 taxonomy shape", () => {
  it("contains exactly 15 categories (OSINT standard)", () => {
    expect(PRIMARY_KINDS).toHaveLength(15);
  });

  it("has no duplicate keys", () => {
    expect(new Set(PRIMARY_KINDS).size).toBe(PRIMARY_KINDS.length);
  });

  it("lists categories in the user-approved display order", () => {
    expect(PRIMARY_KINDS).toEqual([
      "conflict",
      "political",
      "disaster",
      "climate",
      "health",
      "cyber",
      "maritime",
      "aviation",
      "economic",
      "news",
      "humanitarian",
      "holiday",
      "research",
      "wildlife",
      "other",
    ]);
  });

  it("has a color for every primary", () => {
    for (const k of PRIMARY_KINDS) {
      expect(PRIMARY_COLOR[k]).toMatch(/^#[0-9a-f]{6}$/i);
    }
  });
});

describe("primaryOf — L2 → L1 mapping", () => {
  it("consolidates festival holidays into one 'holiday' category", () => {
    expect(primaryOf("holiday")).toBe("holiday");
    expect(primaryOf("festival_holiday_past")).toBe("holiday");
    expect(primaryOf("festival_holiday_imminent_priority")).toBe("holiday");
    expect(primaryOf("festival_holiday_imminent_routine")).toBe("holiday");
    expect(primaryOf("festival_holiday_upcoming")).toBe("holiday");
  });

  it("consolidates air quality + extreme weather into 'climate'", () => {
    expect(primaryOf("air_quality")).toBe("climate");
    expect(primaryOf("weather_extreme")).toBe("climate");
    expect(primaryOf("climate")).toBe("climate");
  });

  it("consolidates news + info variants into 'news'", () => {
    expect(primaryOf("news")).toBe("news");
    expect(primaryOf("news_priority")).toBe("news");
    expect(primaryOf("news_routine")).toBe("news");
    expect(primaryOf("news_info")).toBe("news");
    expect(primaryOf("news_headline")).toBe("news");
    expect(primaryOf("news_significant")).toBe("news");
    expect(primaryOf("news_peripheral")).toBe("news");
  });

  it("re-categorizes HDX HAPI series under 'humanitarian'", () => {
    expect(primaryOf("hdx_hapi_national_risk_priority")).toBe("humanitarian");
    expect(primaryOf("hdx_hapi_national_risk_routine")).toBe("humanitarian");
    expect(primaryOf("hdx_hapi_national_risk_info")).toBe("humanitarian");
    expect(primaryOf("hdx_hapi_funding_priority")).toBe("humanitarian");
    expect(primaryOf("hdx_hapi_funding_routine")).toBe("humanitarian");
    expect(primaryOf("hdx_hapi_funding_info")).toBe("humanitarian");
    expect(primaryOf("hdx_humanitarian")).toBe("humanitarian");
  });

  it("keeps conflict, maritime, aviation, economic as their own categories", () => {
    expect(primaryOf("conflict")).toBe("conflict");
    expect(primaryOf("military")).toBe("conflict");
    expect(primaryOf("maritime")).toBe("maritime");
    expect(primaryOf("chokepoint_tanker_priority")).toBe("maritime");
    expect(primaryOf("vessel_sanctions_red")).toBe("maritime");
    expect(primaryOf("flight")).toBe("aviation");
    expect(primaryOf("economic")).toBe("economic");
    expect(primaryOf("financial")).toBe("economic");
  });

  it("puts disaster (quake + fire + flood) under 'disaster'", () => {
    expect(primaryOf("disaster")).toBe("disaster");
    expect(primaryOf("quake")).toBe("disaster");
    expect(primaryOf("fire")).toBe("disaster");
    expect(primaryOf("fire_extreme")).toBe("disaster");
    expect(primaryOf("fire_high")).toBe("disaster");
    expect(primaryOf("fire_significant")).toBe("disaster");
    expect(primaryOf("fire_top50")).toBe("disaster");
    expect(primaryOf("flood")).toBe("disaster");
    expect(primaryOf("flood_minor")).toBe("disaster");
    expect(primaryOf("flood_moderate")).toBe("disaster");
    expect(primaryOf("flood_major")).toBe("disaster");
    expect(primaryOf("flood_action")).toBe("disaster");
  });

  it("puts health + disease outbreak variants under 'health'", () => {
    expect(primaryOf("health")).toBe("health");
    expect(primaryOf("disease_outbreak_priority")).toBe("health");
    expect(primaryOf("disease_outbreak_routine")).toBe("health");
    expect(primaryOf("disease_outbreak_info")).toBe("health");
  });

  it("puts cyber + threat cluster + scanner + secret-leak + compliance + sanction under 'cyber'", () => {
    expect(primaryOf("cyber")).toBe("cyber");
    expect(primaryOf("sanction")).toBe("cyber");
    expect(primaryOf("threatcluster_threat_critical")).toBe("cyber");
    expect(primaryOf("threatcluster_threat_high")).toBe("cyber");
    expect(primaryOf("threatcluster_threat_low")).toBe("cyber");
    expect(primaryOf("threatcluster_vuln_active")).toBe("cyber");
    expect(primaryOf("threatcluster_vuln_background")).toBe("cyber");
    expect(primaryOf("threatcluster_vuln_critical")).toBe("cyber");
    expect(primaryOf("scanner_malicious")).toBe("cyber");
    expect(primaryOf("scanner_benign")).toBe("cyber");
    expect(primaryOf("scanner_unknown")).toBe("cyber");
    expect(primaryOf("secret_leak_broad")).toBe("cyber");
    expect(primaryOf("secret_leak_triggered")).toBe("cyber");
    expect(primaryOf("secret_leak_ignored")).toBe("cyber");
    expect(primaryOf("secret_leak_info")).toBe("cyber");
    expect(primaryOf("compliance_match_exact_priority")).toBe("cyber");
    expect(primaryOf("compliance_match_partial_routine")).toBe("cyber");
    expect(primaryOf("compliance_match_low_confidence")).toBe("cyber");
  });

  it("puts political + politician trade variants under 'political'", () => {
    expect(primaryOf("political")).toBe("political");
    expect(primaryOf("politician_trade_house_large")).toBe("political");
    expect(primaryOf("politician_trade_senate")).toBe("political");
    expect(primaryOf("politician_trade_info")).toBe("political");
  });

  it("puts research + wildlife under their own categories", () => {
    expect(primaryOf("paper_priority")).toBe("research");
    expect(primaryOf("paper_routine")).toBe("research");
    expect(primaryOf("paper_info")).toBe("research");
    expect(primaryOf("wildlife_priority")).toBe("wildlife");
    expect(primaryOf("wildlife_routine")).toBe("wildlife");
    expect(primaryOf("wildlife_info")).toBe("wildlife");
  });

  it("falls back to 'other' for unknown L2 kinds (defensive)", () => {
    expect(primaryOf("nonexistent_kind_xyz")).toBe("other");
    expect(primaryOf("")).toBe("other");
  });

  it("puts radiation, daylight, other under 'other'", () => {
    expect(primaryOf("radiation")).toBe("other");
    expect(primaryOf("daylight")).toBe("other");
    expect(primaryOf("other")).toBe("other");
  });

  it("every L2 in KIND_TO_PRIMARY maps to one of the 15 primaries", () => {
    for (const [l2, l1] of Object.entries(KIND_TO_PRIMARY)) {
      expect(PRIMARY_KINDS).toContain(l1);
      expect(l2).not.toContain(" "); // sanity — no accidental whitespace
    }
  });
});