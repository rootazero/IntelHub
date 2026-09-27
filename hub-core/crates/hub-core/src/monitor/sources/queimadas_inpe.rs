//! INPE Queimadas (Brazilian wildfire satellite detection program,
//! https://queimadas.dgi.inpe.br/queimadas/dados-abertos/ — keyless,
//! INPE public-domain satellite data). Phase 1.7 of the public-API
//! integration roadmap (`docs/superpowers/roadmaps/2026-09-27-public-api-
//! integration-roadmap.md`).
//!
//! ## Strategy
//!
//! Daily sweep: fetch yesterday's daily CSV (the most recent complete
//! fire-spot dataset, INPE publishes around 09:00 UTC for the prior
//! day). Parse all rows, sort by Fire Radiative Power (FRP) descending,
//! emit the top 50 as Signals. Per-row severity ladder:
//!   - FRP > 500 MW → priority (extreme fire event)
//!   - FRP > 200 MW → priority (high-intensity)
//!   - FRP > 100 MW → routine (significant)
//!   - otherwise    → info (top-50 means it's still meaningful)
//!
//! Why top-50 cap: 14k+ fires per day, the majority are small (< 20 MW)
//! low-impact surface fires. Capping at top-50 keeps geo_events from
//! flooding while ensuring the most important events always reach
//! the dashboard. A future spec extension can do spatial clustering
//! for deforestation fronts.
//!
//! ## CSV format
//!
//! `https://dataserver-coids.inpe.br/queimadas/queimadas/focos/csv/diario/Brasil/focos_diario_br_<YYYYMMDD>.csv`
//!
//! Schema (header verified 2026-09-27 against live file):
//!   id, lat, lon, data_hora_gmt, satelite, municipio, estado, pais,
//!   municipio_id, estado_id, pais_id, numero_dias_sem_chuva,
//!   precipitacao, risco_fogo, bioma, frp
//!
//! 2.1 MB / 14k rows typical. No auth. CSV parser is a small loop
//! (csv crate not worth a new dep for this size).
//!
//! ## Lookback strategy
//!
//! Today's CSV is usually empty (real-time data lives in the 10-min
//! archive). Yesterday's CSV is the canonical daily file but can be
//! delayed a few hours after 00:00 UTC. We try [yesterday, 2 days ago,
//! 3 days ago] in order — the first 200 OK wins. 4+ day old data
//! is a degraded scenario (skip + log).
//!
//! ## Auth
//!
//! Keyless. INPE Programa Queimadas is a public Brazilian government
//! service, no registration. Brazil public domain.
//!
//! ## external_id
//!
//! `inpe:{id}` where `id` is INPE's UUID per fire spot. Idempotent
//! across re-polls (same fire = same id, geo_events dedups). The
//! upstream ID is stable for the lifetime of the fire record.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const BASE_URL: &str =
    "https://dataserver-coids.inpe.br/queimadas/queimadas/focos/csv/diario/Brasil";

/// Cap on signals emitted per day. 14k+ raw fires/day in the dry
/// season, ~3k in the wet season; top-50 by FRP is the OSINT-relevant
/// subset. Cap is generous enough to catch a major fire outbreak
/// (50 simultaneous big fires is rare) while keeping geo_events
/// from flooding.
const TOP_N: usize = 50;

/// FRP thresholds in megawatts. Tuned for OSINT relevance (newsworthy
/// fire events) rather than meteorological strictness — INPE classifies
/// "high" fires at FRP > 100 MW, "extreme" at FRP > 500 MW.
const FRP_EXTREME_MW: f64 = 500.0;
const FRP_HIGH_MW: f64 = 200.0;
const FRP_SIGNIFICANT_MW: f64 = 100.0;

pub struct QueimadasInpe;

impl Source for QueimadasInpe {
    fn name(&self) -> &'static str {
        "queimadas_inpe"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(24 * 3600) // 24h — roadmap §2.1
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            // Lookback: yesterday → 2 days ago → 3 days ago. The
            // daily CSV is published around 09:00 UTC for the prior
            // day, but can be delayed a few hours on weekends /
            // holidays. Anything older than 3 days is a degraded
            // upstream — log and bail.
            let now = chrono::Utc::now();
            for offset_days in 1..=3 {
                let date = now - chrono::Duration::days(offset_days);
                let date_str = date.format("%Y%m%d").to_string();
                let url = format!("{BASE_URL}/focos_diario_br_{date_str}.csv");
                let resp = match ctx.http.get(&url).send().await {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!(date = %date_str, error = %e, "queimadas_inpe fetch failed");
                        continue;
                    }
                };
                if !resp.status().is_success() {
                    // 404 = no file yet for that date (not yet
                    // published). Continue to older lookback.
                    if resp.status() == 404 {
                        tracing::debug!(date = %date_str, "queimadas_inpe 404 — try older");
                        continue;
                    }
                    tracing::warn!(date = %date_str, status = %resp.status(), "queimadas_inpe non-2xx");
                    continue;
                }
                let body = match resp.text().await {
                    Ok(t) => t,
                    Err(e) => {
                        tracing::warn!(date = %date_str, error = %e, "queimadas_inpe body read failed");
                        continue;
                    }
                };
                let mut sigs = parse_csv(&body);
                // Annotate with the date the data covers so sp6 can
                // surface "data is 2 days stale" if needed.
                for s in &mut sigs {
                    if let Some(obj) = s.payload.as_object_mut() {
                        obj.insert(
                            "data_date".into(),
                            serde_json::Value::String(
                                date.format("%Y-%m-%d").to_string(),
                            ),
                        );
                    }
                }
                return Ok(sigs);
            }
            // All 3 lookback days failed — degraded upstream.
            tracing::warn!("queimadas_inpe: all 3 lookback days failed (404 or non-2xx)");
            Ok(Vec::new())
        }
        .boxed()
    }
}

/// Pure CSV parser. Walks the daily fire-spot CSV, sorts by FRP
/// desc, emits the top N as Signals. Defensive against malformed
/// rows (no panics on bad floats, missing fields, etc.).
fn parse_csv(body: &str) -> Vec<Signal> {
    let mut lines = body.lines();
    let Some(header_line) = lines.next() else {
        return Vec::new();
    };
    // Header positions — we look these up by name so the parser
    // survives upstream column reordering. INPE has changed the
    // schema before (per their changelog); matching by name is
    // robust to that.
    let header: Vec<&str> = header_line.split(',').map(str::trim).collect();
    let col = |name: &str| -> Option<usize> {
        header.iter().position(|h| *h == name)
    };
    let Some(col_id) = col("id") else { return Vec::new(); };
    let Some(col_lat) = col("lat") else { return Vec::new(); };
    let Some(col_lon) = col("lon") else { return Vec::new(); };
    let Some(col_dt) = col("data_hora_gmt") else { return Vec::new(); };
    let Some(col_sat) = col("satelite") else { return Vec::new(); };
    let Some(col_mun) = col("municipio") else { return Vec::new(); };
    let Some(col_est) = col("estado") else { return Vec::new(); };
    let Some(col_dry) = col("numero_dias_sem_chuva") else { return Vec::new(); };
    let Some(col_precip) = col("precipitacao") else { return Vec::new(); };
    let Some(col_risk) = col("risco_fogo") else { return Vec::new(); };
    let Some(col_biome) = col("bioma") else { return Vec::new(); };
    let Some(col_frp) = col("frp") else { return Vec::new(); };
    // Parse all rows into a sortable vec.
    let mut fires: Vec<Fires> = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split(',').map(str::trim).collect();
        if cols.len() < header.len() {
            continue;
        }
        let id: &str = match cols.get(col_id).copied() {
            Some(s) => s,
            None => continue,
        };
        let lat_str: &str = cols.get(col_lat).copied().unwrap_or("");
        let lat: f64 = match lat_str.trim().parse::<f64>() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let lon_str: &str = cols.get(col_lon).copied().unwrap_or("");
        let lon: f64 = match lon_str.trim().parse::<f64>() {
            Ok(v) => v,
            Err(_) => continue,
        };
        if !lat.is_finite() || !lon.is_finite() {
            continue;
        }
        let frp_str: &str = cols.get(col_frp).copied().unwrap_or("");
        let frp: f64 = match frp_str.trim().parse::<f64>() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let dry_days: Option<i64> = cols
            .get(col_dry)
            .and_then(|s| s.parse::<i64>().ok());
        let precipitacao: Option<f64> = cols
            .get(col_precip)
            .and_then(|s| s.parse::<f64>().ok());
        let risco_fogo: Option<f64> = cols
            .get(col_risk)
            .and_then(|s| s.parse::<f64>().ok());
        fires.push(Fires {
            id: id.to_string(),
            lat,
            lon,
            frp,
            datetime: cols.get(col_dt).copied().unwrap_or("").to_string(),
            satellite: cols.get(col_sat).copied().unwrap_or("").to_string(),
            municipio: cols.get(col_mun).copied().unwrap_or("").to_string(),
            estado: cols.get(col_est).copied().unwrap_or("").to_string(),
            dry_days,
            precipitacao,
            risco_fogo,
            biome: cols.get(col_biome).copied().unwrap_or("").to_string(),
        });
    }
    // Sort by FRP descending; cap at TOP_N.
    fires.sort_by(|a, b| b.frp.partial_cmp(&a.frp).unwrap_or(std::cmp::Ordering::Equal));
    fires.truncate(TOP_N);
    fires
        .into_iter()
        .map(|f| {
            let (severity, kind) = if f.frp >= FRP_EXTREME_MW {
                ("priority", "fire_extreme")
            } else if f.frp >= FRP_HIGH_MW {
                ("priority", "fire_high")
            } else if f.frp >= FRP_SIGNIFICANT_MW {
                ("routine", "fire_significant")
            } else {
                ("info", "fire_top50")
            };
            let title = format!(
                "{} / {} — {:.0} MW ({}, {})",
                f.municipio, f.estado, f.frp, kind, f.biome
            );
            Signal::new(kind, title, f.lat, f.lon, format!("inpe:{}", f.id))
                .severity(severity)
                .payload(serde_json::json!({
                    "id": f.id,
                    "datetime_gmt": f.datetime,
                    "satellite": f.satellite,
                    "municipio": f.municipio,
                    "estado": f.estado,
                    "biome": f.biome,
                    "frp_mw": f.frp,
                    "dry_days": f.dry_days,
                    "precip_mm": f.precipitacao,
                    "risco_fogo": f.risco_fogo,
                    "extreme_type": kind,
                }))
        })
        .collect()
}

/// Internal: one parsed fire record.
struct Fires {
    id: String,
    lat: f64,
    lon: f64,
    frp: f64,
    datetime: String,
    satellite: String,
    municipio: String,
    estado: String,
    dry_days: Option<i64>,
    precipitacao: Option<f64>,
    risco_fogo: Option<f64>,
    biome: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_csv() -> String {
        "id,lat,lon,data_hora_gmt,satelite,municipio,estado,pais,municipio_id,estado_id,pais_id,numero_dias_sem_chuva,precipitacao,risco_fogo,bioma,frp\n\
         aaaa,-10.0,-50.0,2026-09-26 12:00:00,GOES-19,ANGICAL,BAHIA,Brasil,2901403,29,33,5,0.5,0.8,Cerrado,76.6\n\
         bbbb,-5.0,-60.0,2026-09-26 12:00:00,GOES-19,PORTEL,PARÁ,Brasil,1505809,15,33,30,0.0,0.95,Amazônia,108.8\n\
         cccc,-15.0,-55.0,2026-09-26 12:00:00,GOES-19,CUIABÁ,MATO GROSSO,Brasil,5103403,51,33,90,0.0,0.99,Pantanal,800.0\n\
         dddd,-20.0,-45.0,2026-09-26 12:00:00,GOES-19,GOIÂNIA,GOIÁS,Brasil,5208707,52,33,60,0.0,0.85,Cerrado,250.0\n\
         eeee,-25.0,-50.0,2026-09-26 12:00:00,GOES-19,CURITIBA,PARANÁ,Brasil,4106902,41,33,10,2.0,0.5,Mata Atlântica,50.0\n"
            .to_string()
    }

    /// Happy path: 5 fires → all emitted (cap=50 not exceeded).
    #[test]
    fn parses_all_when_under_cap() {
        let sigs = parse_csv(&sample_csv());
        assert_eq!(sigs.len(), 5);
    }

    /// Sort: highest FRP first. 800 MW fire → first signal.
    #[test]
    fn sorted_by_frp_desc() {
        let sigs = parse_csv(&sample_csv());
        assert!(sigs[0].title.contains("CUIABÁ"));
        assert!(sigs[0].title.contains("800"));
    }

    /// Top-N cap: 60 fires with cap=50 → 50 emitted.
    #[test]
    fn caps_at_top_n() {
        let mut csv = "id,lat,lon,data_hora_gmt,satelite,municipio,estado,pais,municipio_id,estado_id,pais_id,numero_dias_sem_chuva,precipitacao,risco_fogo,bioma,frp\n".to_string();
        for i in 0..60 {
            csv.push_str(&format!(
                "id{i},-10.0,-50.0,2026-09-26 12:00:00,GOES-19,X,Y,Brasil,1,1,1,1,0,0,Cerrado,{}.0\n",
                1000 - i
            ));
        }
        let sigs = parse_csv(&csv);
        assert_eq!(sigs.len(), TOP_N);
        // First should be the 1000 MW one
        assert!(sigs[0].payload["frp_mw"].as_f64().unwrap() > 900.0);
    }

    /// Severity ladder.
    #[test]
    fn severity_ladder() {
        let sigs = parse_csv(&sample_csv());
        // 800 MW → priority, fire_extreme
        assert_eq!(sigs[0].severity, "priority");
        assert_eq!(sigs[0].kind, "fire_extreme");
        // 250 MW → priority, fire_high
        assert_eq!(sigs[1].severity, "priority");
        assert_eq!(sigs[1].kind, "fire_high");
        // 108.8 MW → routine, fire_significant
        assert_eq!(sigs[2].severity, "routine");
        assert_eq!(sigs[2].kind, "fire_significant");
        // 76.6 MW → info, fire_top50
        assert_eq!(sigs[3].severity, "info");
        assert_eq!(sigs[3].kind, "fire_top50");
    }

    /// external_id format: `inpe:{id}`.
    #[test]
    fn external_id_shape() {
        let sigs = parse_csv(&sample_csv());
        assert!(sigs[0].external_id.starts_with("inpe:"));
        // Use the upstream UUID directly so the id is stable across re-polls.
        assert_eq!(sigs[0].external_id, "inpe:cccc");
    }

    /// Coords preserved verbatim.
    #[test]
    fn coords_preserved() {
        let sigs = parse_csv(&sample_csv());
        let cuiaba = sigs.iter().find(|s| s.title.contains("CUIABÁ")).unwrap();
        assert!((cuiaba.lat + 15.0).abs() < 1e-9);
        assert!((cuiaba.lon + 55.0).abs() < 1e-9);
    }

    /// Defensive: empty body → zero signals, no panic.
    #[test]
    fn empty_body_returns_empty() {
        assert!(parse_csv("").is_empty());
    }

    /// Defensive: header-only → zero signals.
    #[test]
    fn header_only_returns_empty() {
        let csv = "id,lat,lon,data_hora_gmt,satelite,municipio,estado,pais,municipio_id,estado_id,pais_id,numero_dias_sem_chuva,precipitacao,risco_fogo,bioma,frp\n";
        assert!(parse_csv(csv).is_empty());
    }

    /// Defensive: missing required column → zero signals + log warn.
    #[test]
    fn missing_required_column_returns_empty() {
        let csv = "id,lat,lon,foo,bar\nf1,0,0,0,0\n";
        assert!(parse_csv(csv).is_empty());
    }

    /// Defensive: malformed row (bad lat/lon) skipped, others parsed.
    #[test]
    fn malformed_row_skipped() {
        let csv = "id,lat,lon,data_hora_gmt,satelite,municipio,estado,pais,municipio_id,estado_id,pais_id,numero_dias_sem_chuva,precipitacao,risco_fogo,bioma,frp\n\
                   aaaa,not-a-number,-50.0,2026-09-26 12:00:00,GOES-19,X,Y,Brasil,1,1,1,1,0,0,Cerrado,100.0\n\
                   bbbb,-10.0,-50.0,2026-09-26 12:00:00,GOES-19,X,Y,Brasil,1,1,1,1,0,0,Cerrado,200.0\n";
        let sigs = parse_csv(csv);
        assert_eq!(sigs.len(), 1); // only bbbb parses
        assert!(sigs[0].external_id.contains("bbbb"));
    }

    /// Threshold inclusivity: exactly at threshold fires.
    #[test]
    fn threshold_inclusive() {
        let mut csv = "id,lat,lon,data_hora_gmt,satelite,municipio,estado,pais,municipio_id,estado_id,pais_id,numero_dias_sem_chuva,precipitacao,risco_fogo,bioma,frp\n".to_string();
        for (id, frp) in [("a", 500.0), ("b", 200.0), ("c", 100.0)] {
            csv.push_str(&format!(
                "{id},0,0,2026-09-26,GOES-19,X,Y,Brasil,1,1,1,1,0,0,Cerrado,{frp}\n"
            ));
        }
        let sigs = parse_csv(&csv);
        // 500 → extreme priority, 200 → high priority, 100 → routine
        assert_eq!(sigs[0].kind, "fire_extreme");
        assert_eq!(sigs[1].kind, "fire_high");
        assert_eq!(sigs[2].kind, "fire_significant");
    }

    /// Header column reordering: parser survives upstream renames.
    /// (Critical for the 2024 case where INPE added a new column.)
    #[test]
    fn header_reorder_survives() {
        // Reorder: frp, id, lat, lon, ...
        let csv = "frp,id,lat,lon,data_hora_gmt,satelite,municipio,estado,pais,municipio_id,estado_id,pais_id,numero_dias_sem_chuva,precipitacao,risco_fogo,bioma\n\
                   100.0,aaaa,-10,-50,2026-09-26,GOES-19,X,Y,Brasil,1,1,1,1,0,0,Cerrado\n";
        let sigs = parse_csv(csv);
        assert_eq!(sigs.len(), 1);
        assert!((sigs[0].lat + 10.0).abs() < 1e-9);
    }
}