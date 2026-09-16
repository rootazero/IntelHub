//! CelesTrak TLE catalog → PG `satellites` (Globe P1). Keyless.
//! ctx.state direct-write precedent (OTX): catalog is not a geo event,
//! so fetch() returns Ok(vec![]) — liveness via health cell + sweephist.

use chrono::{DateTime, TimeZone, Utc};

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
