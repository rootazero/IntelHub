//! ipapi.co IP-geolocation + country-metadata lookup.
//!
//! Free keyless endpoint at `https://ipapi.co/<ip>/json/` (30K
//! requests/month per IP without auth). Returns rich per-IP
//! metadata: city / region / country (with ISO codes + capital) /
//! continent / postal / latitude / longitude / timezone / ASN +
//! organization / currency / languages. Useful for IP geolocation
//! attribution when OSINT intel (OTX / URLhaus / Shodan) flags a
//! hostile IP — knowing the city + country pivots investigation
//! to the relevant RIR / national CERT.
//!
//! Unlike the synthetic-anchor pattern used by AWS / GCP IP-ranges
//! (which have no per-prefix geo), ipapi.co returns actual
//! lat/lon per IP. Each Signal is anchored at the IP's REAL
//! geolocation, which makes the radar precise.
//!
//! Rate limit: 30K/month (no per-second cap stated; 250ms
//! inter-query gap is courteous). Default watchlist = 5
//! well-known IPs (Cloudflare DNS / Google DNS / Quad9 /
//! OpenDNS / GitHub) shipped so day-1 surfaces variety of
//! geo data.

use std::time::Duration;

use chrono::Utc;
use serde::Deserialize;
use serde_json::json;

use crate::monitor::{Ctx, Signal, Source};
use crate::HubError;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(12);
const INTER_QUERY_GAP: Duration = Duration::from_millis(1500);

const DEFAULT_WATCH: &[&str] = &[
    "1.1.1.1",            // Cloudflare DNS
    "8.8.8.8",            // Google DNS
    "140.82.121.4",       // GitHub
];

#[derive(Clone)]
pub struct IpapiCo {
    pub watch: Vec<String>,
}

impl Default for IpapiCo {
    fn default() -> Self {
        Self {
            watch: DEFAULT_WATCH.iter().map(|s| s.to_string()).collect(),
        }
    }
}

#[derive(Deserialize)]
struct IpapiCoResponse {
    ip: String,
    #[serde(default)]
    city: Option<String>,
    #[serde(default)]
    region: Option<String>,
    #[serde(default)]
    region_code: Option<String>,
    #[serde(default)]
    country_name: Option<String>,
    #[serde(default)]
    country_code: Option<String>,
    #[serde(default)]
    country_code_iso3: Option<String>,
    #[serde(default)]
    country_capital: Option<String>,
    #[serde(default)]
    continent_code: Option<String>,
    #[serde(default)]
    postal: Option<String>,
    #[serde(default)]
    latitude: Option<f64>,
    #[serde(default)]
    longitude: Option<f64>,
    #[serde(default)]
    timezone: Option<String>,
    #[serde(default)]
    asn: Option<String>,
    #[serde(default)]
    org: Option<String>,
}

impl Source for IpapiCo {
    fn name(&self) -> &'static str {
        "ipapi_co"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(24 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> futures::future::BoxFuture<'a, Result<Vec<Signal>, HubError>> {
        Box::pin(async move {
            let mut sigs = Vec::new();
            for (i, ip) in self.watch.iter().enumerate() {
                if i > 0 {
                    tokio::time::sleep(INTER_QUERY_GAP).await;
                }
                match query(ctx, ip).await {
                    Ok(Some(sig)) => sigs.push(sig),
                    Ok(None) => {
                        tracing::debug!(ip = %ip, "ipapi_co: no result");
                    }
                    Err(e) => {
                        tracing::warn!(
                            ip = %ip,
                            error = %e,
                            "ipapi_co: query failed",
                        );
                    }
                }
            }
            Ok(sigs)
        })
    }
}

async fn query(ctx: &Ctx, ip: &str) -> Result<Option<Signal>, HubError> {
    let url = format!("https://ipapi.co/{ip}/json/");
    let resp = tokio::time::timeout(
        REQUEST_TIMEOUT,
        ctx.http
            .get(&url)
            .header("Accept", "application/json")
            // ipapi.co TOS requires a real UA with contact info.
            .header(
                "User-Agent",
                "IntelHub-research/1.0 (admin@intelhub.local)",
            )
            .send(),
    )
    .await
    .map_err(|_| HubError::sensor("ipapi_co: request timed out".to_string()))?
    .map_err(|e| HubError::sensor(format!("ipapi_co: {e}")))?;

    if !resp.status().is_success() {
        let s = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snip: String = body.chars().take(160).collect();
        return Err(HubError::sensor(format!(
            "ipapi_co: HTTP {s}: {snip}"
        )));
    }

    let body: IpapiCoResponse = resp
        .json()
        .await
        .map_err(|e| HubError::sensor(format!("ipapi_co: parse: {e}")))?;

    let (lat, lon) = match (body.latitude, body.longitude) {
        (Some(la), Some(lo)) => (la, lo),
        _ => return Ok(None),
    };

    let payload = json!({
        "ip": body.ip,
        "city": body.city,
        "region": body.region,
        "region_code": body.region_code,
        "country_name": body.country_name,
        "country_code": body.country_code,
        "country_code_iso3": body.country_code_iso3,
        "country_capital": body.country_capital,
        "continent_code": body.continent_code,
        "postal": body.postal,
        "timezone": body.timezone,
        "asn": body.asn,
        "org": body.org,
        "looked_up_at": Utc::now().to_rfc3339(),
    });

    let loc_str = format!(
        "{}, {}, {}",
        body.city.clone().unwrap_or_default(),
        body.region.clone().unwrap_or_default(),
        body.country_name.clone().unwrap_or_default(),
    );
    let loc_str = loc_str.trim_matches(',').trim().to_string();

    Ok(Some(
        Signal::new(
            "cyber",
            format!("ipapi.co geolocation: {ip} → {loc_str}"),
            lat,
            lon,
            format!("ipapi_co:{ip}"),
        )
        .payload(payload),
    ))
}