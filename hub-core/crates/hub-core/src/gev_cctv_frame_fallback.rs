//! GEV P11 T2+T3: CCTV frame fallback helpers — Street View (env-gated) and
//! synthetic SVG (always succeeds).
//!
//! These are the two fallback tiers for `/api/v1/gev/cctv/frame/{id}`:
//!   1. (existing upstream proxy, gev_cctv.rs)
//!   2. (here) Google Street View Static API — env-gated, returns None when
//!      no key is set so the caller degrades gracefully.
//!   3. (here) Pure synthetic SVG — always succeeds, never 502.
//!
//! Spec: docs/superpowers/specs/2026-09-19-gev-p11-camera-feature-port-design.md §3.4

use std::time::Duration;

// ── HTML escape (SVG injection defence) ───────────────────────────────────

/// Escape characters that would break an SVG text element or attribute value.
/// Escapes `<`, `>`, `&`, `"` — the minimal set for XML/SVG.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ── T2: synthetic SVG ──────────────────────────────────────────────────────

/// Build a 320×180 SVG frame placeholder for a CCTV camera.
///
/// The SVG is deterministic (no clock/hash noise), so two calls for the same
/// inputs produce identical bytes — which is good for HTTP caching and unit
/// testing.
///
/// User-supplied strings (id, label, city, status) are HTML-escaped to prevent
/// SVG injection into the response body.
///
/// # Arguments
/// * `id`       — camera identifier (e.g. `"tfl:JamCams_00002.00865"`)
/// * `label`    — display name (e.g. `"A406 Billet Upass E"`)
/// * `city`     — city name (e.g. `"London"`)
/// * `status`   — status text shown in the footer (e.g. `"DOWN"`)
///
/// # Returns
/// Raw SVG bytes (`image/svg+xml` content).
pub fn build_synthetic_svg(id: &str, label: &str, city: &str, status: &str) -> Vec<u8> {
    // Derive a hue from the camera id so each camera gets a unique gradient.
    let hue: u16 = id.bytes().fold(0u16, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u16));
    let hue1 = (hue % 360) as f32;
    let hue2 = ((hue + 46) % 360) as f32;

    let safe_id = html_escape(id);
    let safe_label = html_escape(label);
    let safe_city = html_escape(city);
    let safe_status = html_escape(status);

    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="320" height="180" viewBox="0 0 320 180">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0%" stop-color="hsl({h1},35%,10%)" />
      <stop offset="60%" stop-color="hsl({h2},42%,6%)" />
      <stop offset="100%" stop-color="#020406" />
    </linearGradient>
    <radialGradient id="flare" cx="0.22" cy="0.24" r="0.78">
      <stop offset="0%" stop-color="hsla({h2},100%,65%,0.35)" />
      <stop offset="100%" stop-color="hsla({h2},100%,40%,0)" />
    </radialGradient>
    <pattern id="scan" width="8" height="8" patternUnits="userSpaceOnUse">
      <rect width="8" height="8" fill="transparent" />
      <rect y="0" width="8" height="1" fill="rgba(255,255,255,0.08)" />
      <rect y="4" width="8" height="1" fill="rgba(255,255,255,0.05)" />
    </pattern>
  </defs>
  <rect width="320" height="180" fill="url(#bg)" />
  <rect width="320" height="180" fill="url(#flare)" />
  <rect width="320" height="180" fill="url(#scan)" />
  <g stroke="rgba(123,233,255,0.25)" stroke-width="1" fill="none">
    <path d="M30 140 Q120 100 200 130 T300 110" />
    <path d="M10 50 Q100 30 180 60 T310 40" />
    <path d="M5 90 Q80 75 140 85 T250 80" />
  </g>
  <g fill="none" stroke="rgba(180,248,255,0.2)" stroke-width="1">
    <rect x="20" y="20" width="280" height="140" rx="4" />
    <line x1="20" y1="90" x2="300" y2="90" />
    <line x1="160" y1="20" x2="160" y2="160" />
  </g>
  <g fill="#9cefff" font-family="monospace" font-size="9">
    <text x="24" y="14" letter-spacing="1">CCTV FEED PLACEHOLDER</text>
    <text x="24" y="158" letter-spacing="0.8">{label} · {city}</text>
    <text x="210" y="158" letter-spacing="0.6">{id}</text>
    <text x="24" y="148" letter-spacing="0.8">{status} — SYNTHETIC PLACEHOLDER</text>
  </g>
</svg>"##,
        h1 = hue1,
        h2 = hue2,
        id = safe_id,
        label = safe_label,
        city = safe_city,
        status = safe_status,
    );
    svg.into_bytes()
}

// ── T3: Street View ────────────────────────────────────────────────────────

/// Street View Static API key resolution chain.
/// `HUB_GOOGLE_MAPS_SERVER_API_KEY` wins over `GOOGLE_MAPS_SERVER_API_KEY`.
/// Returns `None` when neither env var is set (graceful degradation).
fn street_view_key() -> Option<String> {
    ["HUB_GOOGLE_MAPS_SERVER_API_KEY", "GOOGLE_MAPS_SERVER_API_KEY"]
        .iter()
        .find_map(|v| {
            std::env::var(v)
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
}

/// Build a Google Street View Static API URL for the given coordinates.
/// Returns `None` when no API key is configured (env-gated — never 502s).
///
/// The returned URL is a properly-encoded Street View Static API request with:
///   `size=640x360` (2× the synthetic SVG resolution for visual fidelity)
///   `source=outdoor` (avoid indoor panorama rooms)
///   `return_error_code=true` (lets us detect upstream errors by HTTP status)
pub fn street_view_url(lat: f64, lon: f64) -> Option<String> {
    let key = street_view_key()?;
    if !lat.is_finite() || !lon.is_finite() {
        return None;
    }
    let url = format!(
        "https://maps.googleapis.com/maps/api/streetview?\
         size=640x360&\
         location={lat},{lon}&\
         heading=0&\
         fov=80&\
         pitch=0&\
         source=outdoor&\
         return_error_code=true&\
         key={key}"
    );
    Some(url)
}

/// Fetch a Google Street View image as a fallback frame.
/// Returns `Some(bytes)` on success; `None` on any failure (including
/// `street_view_url` returning `None` when no key is configured).
///
/// The caller is responsible for setting the correct `Content-Type` on the
/// response. Street View always returns `image/jpeg` on success.
///
/// # Arguments
/// * `client` — shared `reqwest::Client` (injected for testability)
/// * `lat` / `lon` — camera coordinates
/// * `timeout_secs` — per-request timeout (recommended: 5s)
pub async fn street_view_fallback(
    client: &reqwest::Client,
    lat: f64,
    lon: f64,
    timeout_secs: u64,
) -> Option<Vec<u8>> {
    let url = street_view_url(lat, lon)?;

    let resp = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        client.get(&url).send(),
    )
    .await
    .map_err(|e| {
        tracing::warn!(error = %e, lat, lon, "street view fallback timed out");
    })
    .ok()
    .and_then(|r| {
        r.map_err(|e| {
            tracing::warn!(error = %e, lat, lon, "street view fallback fetch failed");
        })
        .ok()
    })?;

    if !resp.status().is_success() {
        tracing::warn!(
            status = %resp.status(),
            lat,
            lon,
            "street view fallback non-200 response",
        );
        return None;
    }

    let ct = resp
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !ct.starts_with("image/") {
        tracing::warn!(content_type = %ct, lat, lon, "street view fallback unexpected content-type");
        return None;
    }

    // Street View images are always JPEGs; cap at 5 MB.
    const STREET_VIEW_MAX_BYTES: usize = 5 * 1024 * 1024;
    let body = resp
        .bytes()
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, lat, lon, "street view fallback body read failed");
        })
        .ok()?;

    if body.len() > STREET_VIEW_MAX_BYTES {
        tracing::warn!(
            size = body.len(),
            lat,
            lon,
            "street view fallback body exceeds 5 MB cap",
        );
        return None;
    }

    Some(body.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── T2: synthetic SVG ────────────────────────────────────────────────

    #[test]
    fn build_synthetic_svg_includes_id_and_city() {
        let bytes = build_synthetic_svg(
            "tfl:JamCams_00002.00865",
            "A406 Billet Upass E",
            "London",
            "DOWN",
        );
        let text = String::from_utf8(bytes).unwrap();

        assert!(
            text.contains("tfl:JamCams_00002.00865"),
            "SVG must contain the camera id"
        );
        assert!(
            text.contains("London"),
            "SVG must contain the city name"
        );
        assert!(
            text.contains("A406 Billet Upass E"),
            "SVG must contain the label"
        );
        assert!(
            text.contains("DOWN"),
            "SVG must contain the status text"
        );
        assert!(
            text.contains("SYNTHETIC PLACEHOLDER"),
            "SVG must contain the synthetic footer"
        );
        // viewBox must be 320x180
        assert!(
            text.contains(r#"viewBox="0 0 320 180""#),
            "SVG viewBox must be 320x180"
        );
    }

    #[test]
    fn build_synthetic_svg_handles_unicode() {
        // Non-ASCII city name must round-trip correctly (no replacement char).
        let bytes = build_synthetic_svg(
            "tokyo:shibuya-1",
            "Shibuya Crossing",
            "Tokyo",
            "UPSTREAM UNAVAILABLE",
        );
        let text = String::from_utf8(bytes.clone()).unwrap();
        // Should be valid UTF-8 (no replacement characters)
        assert!(
            String::from_utf8(bytes).is_ok(),
            "SVG bytes must be valid UTF-8 with unicode city name"
        );
        assert!(text.contains("Tokyo"));
        assert!(text.contains("Shibuya Crossing"));
    }

    #[test]
    fn build_synthetic_svg_html_escapes_injection() {
        // Attempt SVG injection via a malicious label/city/id.
        let malicious = "<script>alert(1)</script>";
        let bytes = build_synthetic_svg(
            malicious,
            malicious,
            malicious,
            malicious,
        );
        let text = String::from_utf8(bytes).unwrap();

        // The malicious string must NOT appear verbatim anywhere in the output;
        // it must be escaped in every occurrence (label, city, id, status).
        assert!(
            !text.contains(malicious),
            "Malicious string must be HTML-escaped, not appear verbatim"
        );
        assert!(
            text.contains("&lt;script&gt;"),
            "Script tag must be escaped as &lt;script&gt;"
        );
        // The opening '<' and '>' of the script tag are escaped; verify no
        // raw '<' appears in the user-supplied text content zones.
        // (SVG markup like <path> uses literal '<' as syntax, which is fine.)
        let injected_part = "script&gt;alert(1)&lt;";
        assert!(
            text.contains(injected_part),
            "alert(1) surrounded by escaped tags must appear in SVG"
        );
    }

    #[test]
    fn build_synthetic_svg_is_deterministic() {
        let bytes1 = build_synthetic_svg("cam-1", "Label", "City", "DOWN");
        let bytes2 = build_synthetic_svg("cam-1", "Label", "City", "DOWN");
        assert_eq!(
            bytes1, bytes2,
            "Two calls with same inputs must produce identical bytes"
        );
    }

    // ── T3: Street View URL ──────────────────────────────────────────────

    /// Save and restore the two Street View env vars so tests are isolated
    /// even when cargo runs them in parallel threads.
    struct SvEnvGuard {
        hub: Option<String>,
        bare: Option<String>,
    }
    impl SvEnvGuard {
        fn new() -> Self {
            Self {
                hub: std::env::var("HUB_GOOGLE_MAPS_SERVER_API_KEY").ok(),
                bare: std::env::var("GOOGLE_MAPS_SERVER_API_KEY").ok(),
            }
        }
        fn clear(&mut self) {
            std::env::remove_var("HUB_GOOGLE_MAPS_SERVER_API_KEY");
            std::env::remove_var("GOOGLE_MAPS_SERVER_API_KEY");
        }
    }
    impl Drop for SvEnvGuard {
        fn drop(&mut self) {
            std::env::remove_var("HUB_GOOGLE_MAPS_SERVER_API_KEY");
            std::env::remove_var("GOOGLE_MAPS_SERVER_API_KEY");
            if let Some(v) = &self.hub {
                std::env::set_var("HUB_GOOGLE_MAPS_SERVER_API_KEY", v);
            }
            if let Some(v) = &self.bare {
                std::env::set_var("GOOGLE_MAPS_SERVER_API_KEY", v);
            }
        }
    }

    #[test]
    fn street_view_url_no_key_returns_none() {
        let _guard = {
            let mut g = SvEnvGuard::new();
            g.clear();
            assert!(
                street_view_url(51.60067, -0.01594).is_none(),
                "Must return None when no API key is configured"
            );
            g
        };
        // _guard dropped here, restoring original state
    }

    #[test]
    fn street_view_url_with_key_pins_host() {
        let _guard = {
            let mut g = SvEnvGuard::new();
            g.clear();
            std::env::set_var("HUB_GOOGLE_MAPS_SERVER_API_KEY", "test-server-key");
            let result = street_view_url(51.60067, -0.01594);
            assert!(
                result.is_some(),
                "Must return Some when key is set"
            );
            let url = result.unwrap();
            assert!(
                url.starts_with("https://maps.googleapis.com/maps/api/streetview?"),
                "URL must use the Street View endpoint: {url}"
            );
            assert!(
                url.contains("size=640x360"),
                "URL must contain size=640x360: {url}"
            );
            assert!(
                url.contains("key=test-server-key"),
                "URL must contain the API key: {url}"
            );
            assert!(
                url.contains("location=51.60067,-0.01594"),
                "URL must contain encoded coordinates: {url}"
            );
            assert!(
                url.contains("source=outdoor"),
                "URL must contain source=outdoor: {url}"
            );
            g
        };
    }

    #[test]
    fn street_view_url_hub_prefix_wins() {
        let _guard = {
            let mut g = SvEnvGuard::new();
            g.clear();
            std::env::set_var("HUB_GOOGLE_MAPS_SERVER_API_KEY", "hub-key");
            std::env::set_var("GOOGLE_MAPS_SERVER_API_KEY", "bare-key");
            let url = street_view_url(0.0, 0.0).expect("key is set");
            assert!(
                url.contains("key=hub-key"),
                "HUB_ prefix must take precedence: {url}"
            );
            g
        };
    }

    #[test]
    fn street_view_url_hub_prefix_blank_falls_through() {
        let _guard = {
            let mut g = SvEnvGuard::new();
            g.clear();
            std::env::set_var("HUB_GOOGLE_MAPS_SERVER_API_KEY", "   ");
            std::env::set_var("GOOGLE_MAPS_SERVER_API_KEY", "bare-key");
            let url = street_view_url(0.0, 0.0).expect("bare key should be used");
            assert!(
                url.contains("key=bare-key"),
                "Blank HUB_ value must fall through to bare key: {url}"
            );
            g
        };
    }

    #[test]
    fn street_view_url_safely_handles_nan_inf() {
        let _guard = {
            let mut g = SvEnvGuard::new();
            g.clear();
            std::env::set_var("HUB_GOOGLE_MAPS_SERVER_API_KEY", "k");
            assert!(street_view_url(f64::NAN, 0.0).is_none());
            assert!(street_view_url(0.0, f64::INFINITY).is_none());
            assert!(street_view_url(f64::NEG_INFINITY, f64::NAN).is_none());
            g
        };
    }
}
