//! CelesTrak TLE catalog → PG `satellites` (Globe P1). Keyless.
//! ctx.state direct-write precedent (OTX): catalog is not a geo event,
//! so fetch() returns Ok(vec![]) — liveness via health cell + sweephist.

use chrono::{DateTime, TimeZone, Utc};
use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::{HubError, Result};

use super::super::{Ctx, Signal, Source};

pub struct TleRecord {
    pub norad_id: i64,
    pub name: String,
    pub line1: String,
    pub line2: String,
}

pub fn parse_tle_catalog(text: &str) -> Vec<TleRecord> {
    let mut out = Vec::new();
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    while let Some(name) = lines.next() {
        let (Some(l1), Some(l2)) = (lines.next(), lines.next()) else { break };
        if !l1.starts_with("1 ") || !l2.starts_with("2 ") {
            continue;
        }
        let Some(norad) = l1.get(2..7).and_then(|s| s.trim().parse::<i64>().ok()) else {
            continue;
        };
        out.push(TleRecord {
            norad_id: norad,
            name: name.to_string(),
            line1: l1.to_string(),
            line2: l2.to_string(),
        });
    }
    out
}

/// TLE epoch (line1 cols 19-32, `YYDDD.DDDDDDDD`) → UTC. YY < 57 ⇒ 20YY.
pub fn tle_epoch_to_utc(line1: &str) -> Option<DateTime<Utc>> {
    let f = line1.get(18..32)?.trim();
    let yy: i32 = f.get(0..2)?.parse().ok()?;
    let year = if yy < 57 { 2000 + yy } else { 1900 + yy };
    let doy: f64 = f.get(2..)?.parse().ok()?;
    let days = doy.floor() as i64;
    let frac = doy - days as f64;
    let jan1 = Utc.with_ymd_and_hms(year, 1, 1, 0, 0, 0).single()?;
    Some(jan1 + chrono::Duration::days(days - 1) + chrono::Duration::milliseconds((frac * 86_400_000.0) as i64))
}

pub fn celestrak_groups() -> Vec<String> {
    std::env::var("HUB_CELESTRAK_GROUPS")
        .unwrap_or_else(|_| "stations,visual,weather,gnss,military".to_string())
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub struct Celestrak;

impl Source for Celestrak {
    fn name(&self) -> &'static str {
        "celestrak"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(6 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let groups = celestrak_groups();
            let mut wrote_any = false;
            for g in &groups {
                let url = format!("https://celestrak.org/NORAD/elements/gp.php?GROUP={g}&FORMAT=tle");
                let text = match ctx.http.get(&url).send().await {
                    Ok(r) if r.status().is_success() => match r.text().await {
                        Ok(t) => t,
                        Err(e) => { tracing::warn!(group = g, error = %e, "celestrak body read failed"); continue; }
                    },
                    Ok(r) => { tracing::warn!(group = g, status = %r.status(), "celestrak group failed"); continue; }
                    Err(e) => { tracing::warn!(group = g, error = %e, "celestrak group failed"); continue; }
                };
                let records = parse_tle_catalog(&text);
                if records.is_empty() {
                    tracing::warn!(group = g, "celestrak group parsed empty");
                    continue;
                }
                // Per-category full replace in one tx — decayed sats vanish.
                let mut tx = ctx.state.pg.begin().await
                    .map_err(|e| HubError::sensor(format!("celestrak tx begin: {e}")))?;
                sqlx::query("DELETE FROM satellites WHERE category = $1")
                    .bind(g).execute(&mut *tx).await
                    .map_err(|e| HubError::sensor(format!("celestrak delete {g}: {e}")))?;
                for r in &records {
                    // ON CONFLICT: CelesTrak groups OVERLAP (25544 ISS and 48274
                    // CSS are in both `stations` and `visual`), and the schema's
                    // PK is global (`norad_id`), not (norad_id, category). A plain
                    // INSERT would raise 23505 and abort the tick, permanently
                    // starving every group after the first collision. Upsert keeps
                    // each category's replace atomic and the union complete.
                    sqlx::query(
                        "INSERT INTO satellites (norad_id, name, category, tle_line1, tle_line2, epoch)
                         VALUES ($1, $2, $3, $4, $5, $6)
                         ON CONFLICT (norad_id) DO UPDATE SET
                           name = EXCLUDED.name,
                           category = EXCLUDED.category,
                           tle_line1 = EXCLUDED.tle_line1,
                           tle_line2 = EXCLUDED.tle_line2,
                           epoch = EXCLUDED.epoch,
                           fetched_at = now()",
                    )
                    .bind(r.norad_id as i32)
                    .bind(&r.name)
                    .bind(g)
                    .bind(&r.line1)
                    .bind(&r.line2)
                    .bind(tle_epoch_to_utc(&r.line1).unwrap_or_else(|| {
                        tracing::warn!(norad_id = r.norad_id, name = %r.name,
                            "celestrak TLE epoch unparseable, falling back to now() (upstream data corrupt)");
                        Utc::now()
                    }))
                    .execute(&mut *tx).await
                    .map_err(|e| HubError::sensor(format!("celestrak insert {g}: {e}")))?;
                }
                tx.commit().await
                    .map_err(|e| HubError::sensor(format!("celestrak commit {g}: {e}")))?;
                wrote_any = true;
                tracing::info!(group = g, count = records.len(), "celestrak catalog replaced");
            }
            if !wrote_any {
                return Err(HubError::sensor("celestrak: every group failed this tick"));
            }
            Ok(vec![]) // catalog is not a geo event; liveness via health cell
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ISS: &str = "ISS (ZARYA)\n1 25544U 98067A   26260.51785714  .00012345  00000-0  23456-3 0  9992\n2 25544  51.6400 208.9163 0006317  69.9862  25.2906 15.49560532420999\n";

    #[test]
    fn parses_valid_triples() {
        let r = parse_tle_catalog(ISS);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].norad_id, 25544);
        assert_eq!(r[0].name, "ISS (ZARYA)");
        assert!(r[0].line1.starts_with("1 25544"));
    }

    #[test]
    fn skips_malformed_lines() {
        assert_eq!(parse_tle_catalog("NAME ONLY\n1 25544U broken\n").len(), 0);
        assert_eq!(parse_tle_catalog("").len(), 0);
    }

    #[test]
    fn tle_epoch_parses_yy_ddd() {
        // 26260.51785714 → 2026, day 260 → 2026-09-17
        let e = tle_epoch_to_utc("1 25544U 98067A   26260.51785714  .00012345  00000-0  23456-3 0  9992").unwrap();
        assert_eq!(e.format("%Y-%m-%d").to_string(), "2026-09-17");
    }

    #[test]
    fn default_groups_match_spec() {
        std::env::remove_var("HUB_CELESTRAK_GROUPS");
        assert_eq!(celestrak_groups(), vec!["stations", "visual", "weather", "gnss", "military"]);
    }
}
