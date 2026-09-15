//! SP6: native monitor — hub-core's own signal collectors. Replaces the
//! Crucix black-box container (spec: docs/superpowers/specs/2026-09-10-
//! intelhub-sp6-native-monitor-design.md). Each source is an independent
//! Tokio task with its own cadence, timeout isolation, and backoff; signals
//! flow Signal → geo_events (idempotent) → bus → alerts. Hub stays zero-LLM.

pub mod fd;
pub mod geo;
pub mod limiter;
pub mod scheduler;
pub mod signals;
pub mod sources;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use crate::error::Result;

/// One normalized geographic signal, pre-persistence.
pub struct Signal {
    pub kind: &'static str,
    pub title: String,
    pub lat: f64,
    pub lon: f64,
    pub severity: &'static str, // info | routine | priority | flash
    pub occurred_at: DateTime<Utc>,
    pub external_id: String,
    pub payload: serde_json::Value,
}

impl Signal {
    pub fn new(
        kind: &'static str,
        title: impl Into<String>,
        lat: f64,
        lon: f64,
        external_id: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            title: title.into().chars().take(200).collect(),
            lat,
            lon,
            severity: "info",
            occurred_at: Utc::now(),
            external_id: external_id.into(),
            payload: serde_json::json!({}),
        }
    }
    pub fn severity(mut self, s: &'static str) -> Self {
        self.severity = s;
        self
    }
    pub fn occurred(mut self, t: DateTime<Utc>) -> Self {
        self.occurred_at = t;
        self
    }
    pub fn payload(mut self, p: serde_json::Value) -> Self {
        self.payload = p;
        self
    }
}

/// Shared per-run context handed to every source (dedicated HTTP client with
/// a 25s ceiling — never the 120s state client; a hung upstream must not pin
/// a source task for two minutes).
pub struct Ctx {
    pub http: reqwest::Client,
    pub config: Arc<crate::Config>,
    pub limiter: Arc<limiter::RateLimiter>,
}

impl Ctx {
    pub fn new(config: Arc<crate::Config>) -> Result<Self> {
        // Browser UA: Cloudflare (error 1010) bans bot-signature clients on
        // several collectors' upstreams (acleddata.com confirmed 2026-09 — a
        // browser UA passes, an honest "intelhub-monitor" UA gets 403'd).
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(25))
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36")
            .build()?;
        Ok(Self {
            http,
            config,
            limiter: Arc::new(limiter::RateLimiter::new()),
        })
    }
}

pub trait Source: Send + Sync {
    fn name(&self) -> &'static str;
    fn interval(&self) -> Duration;
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>>;
}

/// SP6B: non-geo collectors (time series / event intel). A collector receives
/// the AppState (series persistence, evidence ingest, alert rules are all
/// DB-touching) and reports (fetched, new) for health accounting.
pub trait SeriesCollector: Send + Sync {
    fn name(&self) -> &'static str;
    fn interval(&self) -> Duration;
    fn collect<'a>(
        &'a self,
        state: &'a crate::state::AppState,
        ctx: &'a Ctx,
    ) -> BoxFuture<'a, Result<(usize, usize)>>;
}

/// Phase-A source set (spec §3). Order defines first-run stagger, not priority.
pub fn registry() -> Vec<Box<dyn Source>> {
    vec![
        Box::new(sources::usgs::Usgs),
        Box::new(sources::noaa::Noaa),
        Box::new(sources::firms::Firms),
        Box::new(sources::gdelt::Gdelt),
        Box::new(sources::opensky::OpenSky),
        Box::new(sources::rss::Rss),
        Box::new(sources::radiation::Radiation),
        // ACLED stays visible on the health board even while its account tier
        // is pending (user decision 2026-09: hiding = forgetting). Hourly
        // retries mean the source SELF-HEALS the day the Access Team approves.
        Box::new(sources::acled::Acled),
        Box::new(sources::kiwisdr::KiwiSdr),
        // SP8 batch-A expansion (Crucix parity): humanitarian + health + cyber.
        Box::new(sources::reliefweb::ReliefWeb),
        Box::new(sources::who::Who),
        Box::new(sources::cisakev::CisaKev),
        // SP8-E cyber plane expansion: NVD 2.0 (full CVE corpus w/ CVSS v3)
        // complements cisakev (actively-exploited subset only). Same
        // honest-DC-metro anchoring pattern.
        Box::new(sources::nvd::Nvd),
        // SP8 batch-B expansion: sanctions tempo + federal contracts + RadNet.
        Box::new(sources::ofac::Ofac),
        Box::new(sources::usaspending::UsaSpending),
        Box::new(sources::epa::Epa),
        // SP8-C social plane: Bluesky (free official API) + Telegram public
        // channels (t.me/s web preview) carry the load; X stays visible-
        // degraded until X_BEARER_TOKEN exists ($200/mo Basic tier).
        Box::new(sources::bluesky::Bluesky),
        Box::new(sources::telegram::TelegramWatch),
        Box::new(sources::x::XWatch),
        // BLS native: unreachable today (Akamai bans all our egress paths)
        // but stays visible per user decision — self-heals when a clean
        // residential-US path + BLS_API_KEY exist. FRED mirrors meanwhile.
        Box::new(sources::bls::Bls),
        // SP8-D climate plane: EONET climate-category events (drought /
        // sea-ice / temp-extremes only — storms/fires/quakes already owned
        // by NOAA/FIRMS/USGS; re-fetching would double-tag).
        Box::new(sources::eonet::Eonet),
    ]
}

/// SP6B series/event collectors (spec §3). Order defines first-run stagger.
pub fn series_registry() -> Vec<Box<dyn SeriesCollector>> {
    vec![
        Box::new(sources::fred::Fred),
        Box::new(sources::eia::Eia),
        Box::new(sources::treasury::Treasury),
        Box::new(sources::markets::Markets),
        Box::new(sources::finintel::FinIntel),
        Box::new(sources::gscpi::Gscpi),
        Box::new(sources::comtrade::Comtrade),
        // SP8-D climate plane: global-warming indicators (NOAA CO2 monthly
        // mean + NASA GISTEMP anomaly). NSIDC ice cut — F5 bot-walled.
        Box::new(sources::climateseries::ClimateSeries),
    ]
}

pub async fn run_monitor(state: crate::state::AppState, ct: CancellationToken) {
    if !state.config.monitor_enabled {
        tracing::info!("monitor disabled via HUB_MONITOR_ENABLED=false");
        return;
    }
    scheduler::run(state, registry(), series_registry(), ct).await
}
