//! ip-api.com IP-geolocation + ASN/ISP lookup.
//!
//! Free keyless endpoint at `http://ip-api.com/json/<ip>`
//! (45 requests/minute from non-localhost IPs without auth).
//! Returns city / region / country / latitude / longitude /
//! timezone / ISP / org / AS. Useful as a SECOND
//! geolocation source alongside ipapi_co: different
//! providers have different SPOFs and data quality — running
//! both gives you cross-validation when attribution
//! matters.
//!
//! Rate limit: 45 req/min strict. 250ms inter-query gap is
//! well under the limit even with the default 5-IP
//! watchlist.
//!
//! ip-api.com runs over HTTP (not HTTPS) on the free tier.
//! The watchlist IPs are well-known so this is acceptable
//! for OSINT intel — the payload data itself doesn't need
//! transport confidentiality (geo + ASN are public).

use std::time::Duration;

use chrono::Utc;
use serde::Deserialize;
use serde_json::json;

use crate::monitor::{Ctx, Signal, Source};
use crate::HubError;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(12);
const INTER_QUERY_GAP: Duration = Duration::from_millis(250);

const DEFAULT_WATCH: &[&str] = &[
    "1.1.1.1",            // Cloudflare DNS
    "8.8.8.8",            // Google DNS
    "9.9.9.9",            // Quad9 DNS
    "208.67.222.222",     // OpenDNS
    "140.82.121.4",       // GitHub
];

#[derive(Clone)]
pub struct IpApiCom {
    pub watch: Vec<String>,
}

impl Default for IpApiCom {
    fn default() -> Self {
        Self {
            watch: DEFAULT_WATCH.iter().map(|s| s.to_string()).collect(),
        }
    }
}

#[derive(Deserialize)]
struct IpApiComResponse {
    status: String,
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    country_code: Option<String>,
    #[serde(default)]
    region: Option<String>,
    #[serde(default)]
    region_name: Option<String>,
    #[serde(default)]
    city: Option<String>,
    #[serde(default)]
    zip: Option<String>,
    #[serde(default)]
    lat: Option<f64>,
    #[serde(default)]
    lon: Option<f64>,
    #[serde(default)]
    timezone: Option<String>,
    #[serde(default)]
    isp: Option<String>,
    #[serde(default)]
    org: Option<String>,
    #[serde(default, rename = "as")]
    as_holder: Option<String>,
    #[serde(default)]
    query: Option<String>,
}

impl Source for IpApiCom {
    fn name(&self) -> &'static str {
        "ip_api_com"
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
                        tracing::debug!(ip = %ip, "ip_api_com: no result");
                    }
                    Err(e) => {
                        tracing::warn!(
                            ip = %ip,
                            error = %e,
                            "ip_api_com: query failed",
                        );
                    }
                }
            }
            Ok(sigs)
        })
    }
}

async fn query(ctx: &Ctx, ip: &str) -> Result<Option<Signal>, HubError> {
    let url = format!("http://ip-api.com/json/{ip}");
    let resp = tokio::time::timeout(
        REQUEST_TIMEOUT,
        ctx.http.get(&url).send(),
    )
    .await
    .map_err(|_| HubError::sensor("ip_api_com: request timed out".to_string()))?
    .map_err(|e| HubError::sensor(format!("ip_api_com: {e}")))?;

    if !resp.status().is_success() {
        let s = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snip: String = body.chars().take(160).collect();
        return Err(HubError::sensor(format!(
            "ip_api_com: HTTP {s}: {snip}"
        )));
    }

    let body: IpApiComResponse = resp
        .json()
        .await
        .map_err(|e| HubError::sensor(format!("ip_api_com: parse: {e}")))?;

    if body.status != "success" {
        return Ok(None);
    }

    let (lat, lon) = match (body.lat, body.lon) {
        (Some(la), Some(lo)) => (la, lo),
        _ => return Ok(None),
    };

    let payload = json!({
        "ip": body.query,
        "city": body.city,
        "region": body.region_name,
        "region_code": body.region,
        "country": body.country,
        "country_code": body.country_code,
        "zip": body.zip,
        "timezone": body.timezone,
        "isp": body.isp,
        "org": body.org,
        "as": body.as_holder,
        "looked_up_at": Utc::now().to_rfc3339(),
    });

    let loc_str = format!(
        "{}, {}, {}",
        body.city.clone().unwrap_or_default(),
        body.region_name.clone().unwrap_or_default(),
        body.country.clone().unwrap_or_default(),
    );
    let loc_str = loc_str.trim_matches(',').trim().to_string();

    Ok(Some(
        Signal::new(
            "cyber",
            format!("ip-api.com geolocation: {ip} → {loc_str}"),
            lat,
            lon,
            format!("ip_api_com:{ip}"),
        )
        .payload(payload),
    ))
}