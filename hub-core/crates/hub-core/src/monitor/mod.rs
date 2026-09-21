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
    /// Shared application state (PG, Redis, Neo4j, etc.). Added 2026-09-16
    /// so Source impls that need DB access (e.g. OTX → create_claim bridge)
    /// can write without going through the ingest path. NOT carried by
    /// SeriesCollector-style collectors — those get `&AppState` directly via
    /// `SeriesCollector::collect`. Series collectors stay cheap.
    pub state: Arc<crate::AppState>,
}

impl Ctx {
    pub fn new(state: Arc<crate::AppState>) -> Result<Self> {
        let config = state.config.clone();
        // Browser UA: Cloudflare (error 1010) bans bot-signature clients on
        // several collectors' upstreams (acleddata.com confirmed 2026-09 — a
        // browser UA passes, an honest "intelhub-monitor" UA gets 403'd).
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(25))
            // Enable gzip end-to-end: send `Accept-Encoding: gzip` so APIs that
            // require it (Digitraffic returns 406 otherwise) accept the request,
            // and auto-decompress the gzipped response so downstream `.text()` /
            // `.json()` see plain bytes.
            .gzip(true)
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36")
            .build()?;
        Ok(Self {
            http,
            config,
            limiter: Arc::new(limiter::RateLimiter::new()),
            state,
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
    let mut out: Vec<Box<dyn Source>> = vec![
        Box::new(sources::usgs::Usgs),
        Box::new(sources::noaa::Noaa),
        Box::new(sources::firms::Firms),
        Box::new(sources::gdelt::Gdelt),
        Box::new(sources::opensky::OpenSky::default()),
        // Globe P1: CelesTrak TLE catalog → PG satellites (dual with adsb.rs;
        // catalog direct-writes via ctx.state, emits no geo Signals).
        Box::new(sources::celestrak::Celestrak),
        // Globe P1: adsb.lol per-aircraft tracks — dual-channel (Redis
        // cumulative snapshot + notable-event Signals). Rate-limit-safe
        // rotation: 1 request/tick over a 14-item queue (~4 req/min, inside
        // measured upstream quota); scheduler backoff is the 429 cooldown.
        Box::new(sources::adsb::Adsb::default()),
        // GEV P13 T4 (2026-09-21): 3rd ADS-B source — six-hub US point sweep
        // (ATL/JFK/ORD/DFW/DEN/PHX, 50nm) → its OWN snapshot at
        // `hub:globe:aircraft:adsbx` (TTL 300s), merged read-time by
        // adsb::merge_globe_snapshots (priority adsb > adsbx > opensky).
        // 300s interval; the sweep paces its six upstream calls through the
        // shared per-host limiter (25s gap) so it adds only ~1.2 req/min to
        // the egress budget the adsb rotation already spends (~4 req/min).
        // Snapshot-only: emits no geo Signals (celestrak/cctv precedent).
        Box::new(sources::adsbexchange::AdsbExchange::default()),
        Box::new(sources::rss::Rss),
        // SP8-E compliance/financial plane expansion: SEC EDGAR EFTS
        // for material-event filings (default form=8-K). Fills the gap
        // finintel (finnhub + stocktwits) and fd.rs (financialdatasets.ai)
        // leave open: the canonical SEC filing record itself. Free, no
        // key, only requires User-Agent contact email (secrets.env).
        Box::new(sources::sec_edgar::SecEdgar),
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
        // SP8-E cyber plane expansion: OSV.dev (open-source ecosystem
        // vuln DB) complements NVD (general CVE) and cisakev (KEV). Same
        // kind=cyber, distinct anchor (Mountain View CA vs Gaithersburg
        // MD vs Washington DC). Free, no key.
        Box::new(sources::osv::Osv),
        // SP8 batch-B expansion: sanctions tempo + federal contracts + RadNet.
        Box::new(sources::ofac::Ofac),
        // SP8-E compliance plane expansion: OpenSanctions tempo signal
        // complements ofac (US SDN only). Same kind=sanction, same DC
        // anchor, dedup on dataset last_change timestamp. Shelved-by-design
        // when HUB_OPENSANCTIONS_API_KEY unset (free tier is key-gated).
        Box::new(sources::opensanctions::OpenSanctions),
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
        // OSINT Framework bridge: Etherscan large-tx / sanctioned-address
        // watch. Free tier keyless (1 req/5s), 6h cadence over the default
        // 3-address watchlist (Tornado router + Binance + Coinbase hot).
        // Self-degrades to "ok/0 new" when no deltas land in the window.
        Box::new(sources::etherscan::Etherscan),
        // OSINT Framework bridge: DefiLlama TVL anomaly detector.
        // Emits financial Signals when a watched protocol's 24h TVL
        // change crosses 15% (priority) / 30% (flash) — catches rug
        // pulls and exploits within their propagation window. Keyless,
        // anchored at protocol HQ (Aave→London, Uniswap→NYC, etc.).
        Box::new(sources::defillama::DefiLlama),
        // OSINT Framework bridge: AlienVault OTX community threat-intel
        // pulses. Anchored at AT&T AlienVault HQ (San Mateo CA) — distinct
        // from cisakev/nvd/ofac DC cluster. Public /pulses/subscribed is
        // keyless; if the upstream returns 401/403 we self-degrade to empty
        // signals without failing the sweep.
        Box::new(sources::otx::Otx),
        // OSINT Framework bridge: urlscan.io live URL-scan search feed.
        // Keyless (~100/day hard cap; with URLSCAN_API_KEY = 5k/day).
        // Anchored at Berlin (urlscan.io operator).
        Box::new(sources::urlscan::Urlscan),
        // OSINT Framework bridge: Global Fishing Watch vessel events.
        // Real vessel coordinates (not HQ anchor) — surfaced in the
        // maritime transport cluster. Shelved by design when GFW_API_TOKEN
        // is unset (free token via globalfishingwatch.org/our-apis).
        Box::new(sources::gfw::Gfw),
        // OSINT Framework bridge 2 (2026-09-16): Overpass (OpenStreetMap
        // POI watcher) + Ahmia (tor hidden-service search) + OpenCorporates
        // (global company registry). Overpass/Ahmia are keyless with
        // graceful degradation on rate-limit; OpenCorp needs a free
        // OPENCORP_API_TOKEN (shelved-by-design without). See each
        // module's docstring for the exact self-heal trigger.
        Box::new(sources::overpass::Overpass),
        Box::new(sources::ahmia::Ahmia),
        Box::new(sources::opencorp::Opencorp),
        // Wikidata: free, keyless counterpart to OpenCorporates for
        // the same OSINT Framework "Business Records → Entities" gap.
        // Same kind=financial; uses real HQ coordinates (P625) so
        // entities land on their actual map location rather than the
        // jurisdiction capital fallback opencorp uses.
        Box::new(sources::wikidata::Wikidata),
        // OSINT Framework bridge 3 (2026-09-17): CourtListener
        // (Public Records → Court Filings, free + keyless) +
        // Leaksify (Email/Breach, free + keyless). Two new gaps
        // filled in one PR — both use Source pattern, both feed
        // existing visual clusters (sanction, cyber) without
        // introducing new infrastructure.
        Box::new(sources::courtlistener::Courtlistener),
        Box::new(sources::leaksify::Leaksify),
        // OSINT Framework bridge 4 (2026-09-17): TorExit (Tor exit-node
        // daily bulk list, free+keyless) + IPsum (stamparm's community
        // threat-IP aggregator feed, free+keyless). Both complement the
        // existing cyber cluster (otx/urlscan/ahmia/leaksify) — tor
        // exit IPs are scanner/credential-stuff/bot origins; ipsum is
        // pre-emptive IP-level blocklist intel. Anchored at operator
        // home country (honest "no per-event geo" stand-in) so they
        // visually cluster together with other operator-HQ-anchored
        // cyber sources.
        Box::new(sources::tor_exit::TorExit),
        Box::new(sources::ipsum::Ipsum),
        // OSINT Framework bridge 5 (2026-09-17): crtsh (Certificate
        // Transparency log search, free+keyless) + openphish (phishing
        // URL catalog, free+keyless — PhishTank public feed was
        // retired in 2024) + shodan_internetdb (free keyless per-IP
        // enrichment; complements paid Shodan with a public data
        // subset). All three fill distinct OSINT Framework gaps:
        // crtsh → "Domain → Certificate Search", openphish → "URL →
        // Phishing", shodan_internetdb → "IP → Shodan InternetDB".
        Box::new(sources::crtsh::CrtSh),
        Box::new(sources::openphish::OpenPhish),
        Box::new(sources::shodan_internetdb::ShodanInternetDb),
        // OSINT Framework bridge 6 (2026-09-17): Spamhaus DROP
        // (authoritative netblock blocklist, free keyless) +
        // blocklist.de (German fail2ban community per-attack-type
        // IP blocklists, free keyless). RDAP originally proposed
        // but Verisign RDAP rejects reqwest's HTTP/2 + default
        // reqwest/0.x User-Agent with HTTP 400 from datacenter
        // egress; curl with HTTP/1.1 + browser UA works. Tracking
        // in separate branch when reqwest <-> Verisign interop is
        // resolved. ThreatMiner blocked from datacenter IP (000);
        // substituted with blocklist.de (per-IP blocklist with
        // fail2ban provenance).
        Box::new(sources::spamhaus_drop::SpamhausDrop),
        Box::new(sources::blocklist_de::BlocklistDe),
        // OSINT Framework bridge 7 (2026-09-17): three free
        // keyless collectors that fill three distinct OSINT
        // gaps. Nominatim (OpenStreetMap reverse geocoding
        // for threat-actor HQ anchoring) + RIPEstat abuse-
        // contact-finder (IRIR abuse emails for incident
        // response) + Internet Archive Wayback Machine (URL
        // archive snapshots for phishing forensics).
        Box::new(sources::nominatim::Nominatim::default()),
        Box::new(sources::ripestat::Ripestat::default()),
        Box::new(sources::wayback::Wayback::default()),
        // OSINT Framework bridge 8 (2026-09-17): three free
        // keyless network-attribution collectors that fill
        // the cloud-IP + AS-topology attribution gaps.
        // aws_ip_ranges (AWS public IP-range feed) +
        // gcp_ip_ranges (GCP public IP-range feed) +
        // ripe_as_overview (RIPE stat per-ASN holder lookup).
        Box::new(sources::aws_ip_ranges::AWSIpRanges),
        Box::new(sources::gcp_ip_ranges::GCPIPRanges),
        Box::new(sources::ripe_as_overview::RipeAsOverview::default()),
        // OSINT Framework bridge 9 (2026-09-17): three free
        // keyless per-IP attribution collectors that fill
        // the granular-IP-intel gap. ipapi_co (rich IP
        // metadata: city/country/lat/lon/ASN/org) +
        // ip_api_com (IP geolocation + ASN/ISP, redundant
        // with ipapi_co for cross-validation) +
        // ripe_prefix_overview (RIPE stat per-prefix BGP
        // info: prefix, AS path, RPKI status).
        Box::new(sources::ipapi_co::IpapiCo::default()),
        Box::new(sources::ip_api_com::IpApiCom::default()),
        Box::new(sources::ripe_prefix_overview::RipePrefixOverview::default()),
        // OSINT Framework bridge 10 (2026-09-17): three free
        // keyless sentinel / active-threat / curated-
        // blocklist collectors that fill the "sentinel
        // domain cross-check" + "active malware URL feed" +
        // "high-quality IP blocklist" gaps.
        // misp_dynamic_dns (MISP-maintained 45K-domain
        // sentinel list of dynamic-DNS providers, used for
        // OSINT false-positive suppression) +
        // urlhaus (abuse.ch plain-text malware/phishing URL
        // feed, ~50K active URLs) + firehol_level1 (curated
        // high-quality IP blocklist, ~4,718 CIDR entries
        // from FireHOL's aggregation of ~30 sources).
        Box::new(sources::misp_dynamic_dns::MispDynamicDns),
        Box::new(sources::urlhaus::Urlhaus),
        Box::new(sources::firehol_level1::FireholLevel1),
        // OSINT Framework bridge 11 (2026-09-17): three free
        // keyless sentinel + rich-Tor collectors that fill
        // the RFC-special-use sentinel cross-check gaps +
        // provide Tor-relay-fingerprint pivots. misp_rfc5735
        // (RFC 5735 Special-Use IPv4 addresses) +
        // misp_rfc6761 (RFC 6761 Special-Use Domain Names) +
        // tor_exit_details (rich Tor exit-addresses feed
        // with fingerprint + Published + LastStatus +
        // ExitAddress timestamps — complementary to existing
        // tor_exit which uses the bare-IP torbulkexitlist).
        Box::new(sources::misp_rfc5735::MispRfc5735),
        Box::new(sources::misp_rfc6761::MispRfc6761),
        Box::new(sources::tor_exit_details::TorExitDetails),
        // OSINT Framework bridge 12 (2026-09-17): three free
        // keyless collectors that fill OSINT Framework gaps —
        // romainmarcoux_malicious_ip (40K most-malicious IPs
        // aggregator, metadata-pattern) + ihr_hegemony (IIJ
        // Lab REST API tracking AS customer-cone reach for
        // internet-topology shift detection — default watch
        // Cloudflare 13335 + Akamai 20940, 100+ dependents
        // each) + misp_second_level_tlds (MISP sentinel of
        // 10,315 Mozilla-PSL 2nd-level TLDs for OSINT false-
        // positive suppression on hostname indicators).
        Box::new(sources::romainmarcoux_malicious_ip::RomainmarcouxMaliciousIp),
        Box::new(sources::ihr_hegemony::IhrHegemony::default()),
        Box::new(sources::misp_second_level_tlds::MispSecondLevelTlds),
    ];
    // GEV P3 (2026-09-17): military installations — Overpass global
    // ["military"] harvest → PG military_installations (contracts.md §4).
    // Keyless, 24h, 4 quadrants with ≥60s politeness + 2× backoff retries;
    // stale sweep only after a full 4/4 round; hub-restart rounds skip when
    // data is <24h old (max(fetched_at) probe). Emits no geo Signals.
    out.push(Box::new(sources::installations::Installations));
    // GEV P3 (2026-09-17): CCTV static catalog base load (T8) — vendor
    // cctv_sources.*.json → PG cctv_cameras (0021, contracts.md §3).
    // Keyless, idempotent; missing vendor dir degrades to a warn, never
    // a boot failure. Emits no geo Signals (catalog, celestrak precedent).
    out.push(Box::new(sources::cctv::CctvLoader));
    // GEV P3 (2026-09-17): CCTV live city providers (T9) — keyless TfL
    // JamCams + Ontario 511 catalogs refresh hourly (provider-scoped
    // stale sweep; a failed provider keeps its rows + error health cell).
    // cctv-health probes a rotating 3-per-provider frame sample every
    // 5min, annotating health_status/health_checked_at. NYC/LTA = T10.
    out.push(Box::new(sources::cctv::refresh::CctvRefresh));
    out.push(Box::new(sources::cctv::refresh::CctvHealth));
    // GEV P3 (2026-09-17): AISStream live vessels — env-gated, opensky
    // pattern. Without AISSTREAM_API_KEY the collector is NOT registered
    // (one startup log line); the T2 REST layer answers the contract's
    // status:"missing-key" envelope (vendor chip copy, contracts.md §5).
    match sources::ais::api_key() {
        Some(_) => out.push(Box::new(sources::ais::Ais::default())),
        None => tracing::info!(
            "ais: no AISSTREAM_API_KEY — collector not registered; /api/ais-live will report status 'missing-key'"
        ),
    }
    out
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
