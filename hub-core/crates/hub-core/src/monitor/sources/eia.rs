//! EIA v2 petroleum spot prices (spec §3): WTI + Brent daily. Key already
//! lives in VM secrets from the Crucix era. Series names carry units.

use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use futures::future::BoxFuture;

use crate::error::Result;
use crate::state::AppState;

use super::super::signals::Observation;
use super::super::{Ctx, SeriesCollector};

const FACETS: &[(&str, &str)] = &[
    ("RWTC", "eia:WTI_SPOT_USD_BBL"),
    ("RBRTE", "eia:BRENT_SPOT_USD_BBL"),
];

pub struct Eia;

impl SeriesCollector for Eia {
    fn name(&self) -> &'static str {
        "eia"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(6 * 3600)
    }
    fn collect<'a>(&'a self, state: &'a AppState, ctx: &'a Ctx) -> BoxFuture<'a, Result<(usize, usize)>> {
        Box::pin(async move {
            let Some(key) = ctx.config.eia_api_key.clone() else {
                tracing::warn!("eia: no EIA_API_KEY — collector degraded by design");
                return Ok((0, 0));
            };
            let mut all = Vec::new();
            for (facet, hub_name) in FACETS {
                let url = format!(
                    "https://api.eia.gov/v2/petroleum/pri/spt/data/?api_key={key}\
                     &frequency=daily&data[0]=value&facets[series][]={facet}\
                     &sort[0][column]=period&sort[0][direction]=desc&length=5"
                );
                match ctx.http.get(&url).send().await {
                    Ok(r) if r.status().is_success() => {
                        let body = r.text().await.unwrap_or_default();
                        all.extend(parse(&body, hub_name));
                    }
                    Ok(r) => tracing::warn!(facet, status = %r.status(), "eia http"),
                    Err(e) => tracing::warn!(facet, error = %e, "eia fetch"),
                }
            }
            let fetched = all.len();
            let new = super::super::signals::persist_observations(state, "monitor:eia", all).await?;
            Ok((fetched, new))
        })
    }
}

pub fn parse(body: &str, hub_series: &str) -> Vec<Observation> {
    let v: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    v.pointer("/response/data")
        .and_then(|d| d.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|row| {
                    let period = row.get("period")?.as_str()?;
                    let d = NaiveDate::parse_from_str(period, "%Y-%m-%d").ok()?;
                    let dt = DateTime::from_naive_utc_and_offset(d.and_hms_opt(0, 0, 0)?, Utc);
                    // EIA v2 renders values as JSON strings ("91.48") — accept both.
                    let value = match row.get("value")? {
                        serde_json::Value::Number(n) => n.as_f64()?,
                        serde_json::Value::String(s) => s.parse().ok()?,
                        _ => return None,
                    };
                    Some(Observation::new(hub_series, dt, value))
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_v2_rows() {
        let body = r#"{"response":{"data":[
            {"period":"2026-09-08","value":"65.31","series":"RWTC"},
            {"period":"2026-09-05","value":64.9,"series":"RWTC"}]}}"#;
        let pts = parse(body, "eia:WTI_SPOT_USD_BBL");
        assert_eq!(pts.len(), 2);
        assert!((pts[0].value - 65.31).abs() < 1e-9);
        assert_eq!(pts[0].series, "eia:WTI_SPOT_USD_BBL");
    }
}
