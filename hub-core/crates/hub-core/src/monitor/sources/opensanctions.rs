//! OpenSanctions tempo signal — the "default" dataset aggregates 30+
//! global sanctions/PEP lists (OFAC SDN, EU CFSP, UN Consolidated, UK
//! HMT, INTERPOL Red Notices, national PEP registers, etc.) into one
//! canonical entity table under ODbL. Free tier requires an API key;
//! without it the source is shelved-by-design (registered in health,
//! produces no signals) — same pattern as firms/reliefweb. Anchored at
//! US Treasury DC, identical visual neighbourhood to ofac's anchor;
//! operators learn "two sanctions-tempo signals in DC = expected".
//! Entity-level work (per-sanctioned-person lookup) belongs to the
//! graph/MCP plane, not the Radar map.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::{HubError, Result};

use super::super::{Ctx, Signal, Source};

const API_BASE: &str = "https://api.opensanctions.org";
const TREASURY_DC: (f64, f64) = (38.8951, -77.0364);

pub struct OpenSanctions;

impl Source for OpenSanctions {
    fn name(&self) -> &'static str {
        "opensanctions"
    }
    fn interval(&self) -> Duration {
        // OpenSanctions dataset refreshes daily; 24h cadence matches
        // upstream rhythm without hammering the API.
        Duration::from_secs(24 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            // Free tier is API-key gated — without it we surface as
            // "shelved-by-design" so operators see the source on the
            // health board but no signals land. Mirrors firms/reliefweb.
            let Some(key) = &ctx.config.monitor_opensanctions_api_key else {
                return Ok(Vec::new());
            };

            // /datasets/{name} returns metadata: entity_count + last_change
            // (ISO 8601 timestamp of the latest materialised delta). Used
            // as the dedup key so re-fetching the same delta emits nothing.
            let url = format!("{API_BASE}/datasets/default");
            let resp = ctx
                .http
                .get(&url)
                .header("Authorization", format!("ApiKey {key}"))
                .send()
                .await?;
            if !resp.status().is_success() {
                return Err(HubError::sensor(format!(
                    "OpenSanctions HTTP {}",
                    resp.status()
                )));
            }
            let j: serde_json::Value = resp.json().await?;

            let entity_count = j
                .get("entity_count")
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            let last_change = j
                .get("last_change")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            if last_change.is_empty() {
                return Err(HubError::sensor(
                    "OpenSanctions: response missing last_change",
                ));
            }

            let title = format!(
                "OpenSanctions default dataset refreshed — {entity_count} entities (30+ lists)"
            );
            Ok(vec![Signal::new(
                "sanction",
                title,
                TREASURY_DC.0,
                TREASURY_DC.1,
                format!("opensanctions:default:{last_change}"),
            )
            .severity("routine")
            .payload(serde_json::json!({
                "dataset": "default",
                "entity_count": entity_count,
                "last_change": last_change,
                "url": "https://www.opensanctions.org/datasets/default/",
            }))])
        }
        .boxed()
    }
}