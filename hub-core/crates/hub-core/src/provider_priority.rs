//! GEV P3 T15: cross-domain provider priority — user directive
//! (2026-09-17): free data sources first, paid-source interfaces reserved.
//!
//! Each dynamic data domain has a provider list. The default order is
//! free-first; `{DOMAIN}_PROVIDER_PRIORITY` (comma-separated provider ids,
//! `HUB_` prefix also honored) overrides it. Paid providers exist as
//! **reserved slots** (`PaidStub`): they participate in priority parsing
//! and validation but their fetch returns `HubError::sensor("not
//! licensed")` until a licensed implementation lands. This keeps the
//! wiring honest without speculative paid implementations.
//!
//! Domains and their providers:
//! - flights: `adsb_lol` (free, live), `opensky` (free OAuth, env-gated),
//!   `adsb_exchange` (paid slot)
//! - vessels: `aisstream` (free, env-gated), `marinetraffic` (paid slot)
//! - traffic: `osm_overpass` (free), `tomtom` (free tier BYOK, env-gated),
//!   `here` (paid slot)

use std::collections::HashSet;

/// A domain's known providers, in default (free-first) priority order.
pub struct DomainProviders {
    pub domain: &'static str,
    pub env_var: &'static str,
    pub default_order: &'static [&'static str],
    /// Reserved paid slots — valid ids for the priority env, but not in
    /// the default order (they never activate unlicensed).
    pub paid_slots: &'static [&'static str],
}

pub const DOMAINS: &[DomainProviders] = &[
    DomainProviders {
        domain: "flights",
        env_var: "FLIGHTS_PROVIDER_PRIORITY",
        default_order: &["adsb_lol", "opensky"],
        paid_slots: &["adsb_exchange"],
    },
    DomainProviders {
        domain: "vessels",
        env_var: "VESSELS_PROVIDER_PRIORITY",
        default_order: &["aisstream"],
        paid_slots: &["marinetraffic"],
    },
    DomainProviders {
        domain: "traffic",
        env_var: "TRAFFIC_PROVIDER_PRIORITY",
        default_order: &["osm_overpass", "tomtom"],
        paid_slots: &["here"],
    },
];

/// Dual-name env read (`HUB_` prefix wins; blank treated as unset —
/// opensky.rs / 410-empty-keys precedent).
fn env_dual(name: &str) -> Option<String> {
    [format!("HUB_{name}"), name.to_string()].iter().find_map(|v| {
        std::env::var(v)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    })
}

/// Resolve a domain's provider priority: env override (validated against
/// known ids, unknown ids rejected) or the free-first default. Paid slots
/// are valid in the override but never in the default.
pub fn provider_priority(domain: &DomainProviders) -> Vec<String> {
    let known: HashSet<&str> = domain
        .default_order
        .iter()
        .chain(domain.paid_slots.iter())
        .copied()
        .collect();
    match env_dual(domain.env_var) {
        Some(raw) => {
            let picked: Vec<String> = raw
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .filter(|s| {
                    if known.contains(s.as_str()) {
                        true
                    } else {
                        tracing::warn!(
                            domain = domain.domain,
                            provider = %s,
                            "unknown provider in priority override — ignored"
                        );
                        false
                    }
                })
                .collect();
            if picked.is_empty() {
                domain.default_order.iter().map(|s| s.to_string()).collect()
            } else {
                picked
            }
        }
        None => domain.default_order.iter().map(|s| s.to_string()).collect(),
    }
}

/// Paid-slot seam: licensed implementations replace this. Until then the
/// error is explicit rather than a silent empty result (failed sources
/// stay visible).
pub fn paid_slot_not_licensed(domain: &str, provider: &str) -> crate::error::HubError {
    crate::error::HubError::sensor(format!(
        "{domain}: provider '{provider}' is a reserved paid slot — not licensed"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests mutate process env; Rust runs tests in parallel threads, so
    /// serialize every env-touching test on one lock.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn flights() -> &'static DomainProviders {
        &DOMAINS[0]
    }

    #[test]
    fn default_is_free_first() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::remove_var("HUB_FLIGHTS_PROVIDER_PRIORITY");
        std::env::remove_var("FLIGHTS_PROVIDER_PRIORITY");
        assert_eq!(provider_priority(flights()), vec!["adsb_lol", "opensky"]);
        // paid slots never in the default
        for d in DOMAINS {
            let def = provider_priority(d);
            for paid in d.paid_slots {
                assert!(!def.contains(&paid.to_string()), "{} default must not contain {paid}", d.domain);
            }
        }
    }

    #[test]
    fn override_reorders_and_validates() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::set_var("FLIGHTS_PROVIDER_PRIORITY", "opensky, adsb_lol ");
        assert_eq!(provider_priority(flights()), vec!["opensky", "adsb_lol"]);
        // unknown ids rejected with a warn, not silently accepted
        std::env::set_var("FLIGHTS_PROVIDER_PRIORITY", "bogus,opensky");
        assert_eq!(provider_priority(flights()), vec!["opensky"]);
        // all-unknown override falls back to the default
        std::env::set_var("FLIGHTS_PROVIDER_PRIORITY", "bogus");
        assert_eq!(provider_priority(flights()), vec!["adsb_lol", "opensky"]);
        // paid slot is a valid override id
        std::env::set_var("FLIGHTS_PROVIDER_PRIORITY", "adsb_exchange,adsb_lol");
        assert_eq!(provider_priority(flights()), vec!["adsb_exchange", "adsb_lol"]);
        std::env::remove_var("FLIGHTS_PROVIDER_PRIORITY");
    }

    #[test]
    fn hub_prefix_wins_and_blank_is_unset() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::set_var("FLIGHTS_PROVIDER_PRIORITY", "opensky");
        std::env::set_var("HUB_FLIGHTS_PROVIDER_PRIORITY", "aisstream-bogus,adsb_lol");
        assert_eq!(provider_priority(flights()), vec!["adsb_lol"]);
        std::env::set_var("HUB_FLIGHTS_PROVIDER_PRIORITY", "   ");
        assert_eq!(provider_priority(flights()), vec!["opensky"]);
        std::env::remove_var("HUB_FLIGHTS_PROVIDER_PRIORITY");
        std::env::remove_var("FLIGHTS_PROVIDER_PRIORITY");
    }

    #[test]
    fn paid_slot_error_is_explicit() {
        let e = paid_slot_not_licensed("vessels", "marinetraffic");
        assert!(e.to_string().contains("not licensed"));
    }
}
