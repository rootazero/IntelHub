//! Central env-backed config resolution for GEV upstream credentials/headers.
//!
//! Kept separate from the modules that consume these values so every
//! key-like env var has one canonical read path. Resolution follows the
//! `tomtom_api_key()` precedent (gev_traffic.rs): the `HUB_`-prefixed name
//! wins over the bare name, empty values are ignored (secrets.env templates
//! ship keys with empty values — the 410-empty-keys incident). Resolved
//! values are never logged.

/// NOAA public API (api.weather.gov) requires a `User-Agent` header — it
/// 403s anonymous/default-UA clients. No key is involved, but the header
/// value is deployment-specific (NOAA asks for a contact in the UA), so it
/// is env-configurable. Resolution order:
///   1. `HUB_NOAA_UA`
///   2. `NOAA_USER_AGENT`
///   3. `"IntelHub/dev"`
pub fn noaa_user_agent() -> String {
    for var in ["HUB_NOAA_UA", "NOAA_USER_AGENT"] {
        if let Ok(v) = std::env::var(var) {
            let v = v.trim();
            if !v.is_empty() {
                return v.to_string();
            }
        }
    }
    "IntelHub/dev".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noaa_ua_precedence_and_default() {
        // Serialized within one test fn to avoid parallel env races.
        std::env::remove_var("HUB_NOAA_UA");
        std::env::remove_var("NOAA_USER_AGENT");
        assert_eq!(noaa_user_agent(), "IntelHub/dev", "unset → default");

        std::env::set_var("NOAA_USER_AGENT", "bare-ua");
        assert_eq!(noaa_user_agent(), "bare-ua", "bare NOAA_USER_AGENT accepted");

        std::env::set_var("HUB_NOAA_UA", "hub-ua");
        assert_eq!(noaa_user_agent(), "hub-ua", "HUB_ prefix wins when both set");

        std::env::set_var("HUB_NOAA_UA", "  ");
        assert_eq!(
            noaa_user_agent(),
            "bare-ua",
            "blank HUB_ value ignored (empty-keys incident)"
        );

        std::env::remove_var("HUB_NOAA_UA");
        std::env::remove_var("NOAA_USER_AGENT");
    }
}
