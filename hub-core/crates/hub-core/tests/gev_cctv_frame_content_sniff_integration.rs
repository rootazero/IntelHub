//! GEV P11 T12 (P12 follow-up): integration tests for the CCTV frame
//! proxy's content-sniff guard.
//!
//! Mirrors the upstream `gods-eye-view/server/providers/cctv/media.js`
//! behavior:
//! - TxDOT ITS endpoints return `application/json` envelopes; the proxy
//!   must decode the base64 `snippet` and serve the JPEG bytes.
//! - Non-image upstreams (NSW's `text/html` 404 page, etc.) must be
//!   rejected at the guard so the browser never sees a non-image body
//!   carrying the `image/jpeg` content-type label.
//!
//! Tests use wiremock to stand in for the upstream camera hosts (real
//! network calls would tie CI to those hosts' availability). The fetch
//! helpers (`fetch_txdot_envelope_via_proxy`, `fetch_image_via_proxy`)
//! take a URL string, so the wiremock URI is passed straight through
//! the production `cctv_http` client — no production code change was
//! needed to make this testable.

use hub_core::gev_cctv::{
    fetch_image_via_proxy, fetch_txdot_envelope_via_proxy, parse_txdot_envelope,
};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Realistic JPEG header bytes (FFD8FFE0 ... FFD9). Long enough for the
/// `accept_upstream_body_for_browser` magic-byte check and the body
/// size cap to never trip on the test fixture.
fn fake_jpeg(size: usize) -> Vec<u8> {
    let mut buf = vec![0xFF, 0xD8, 0xFF, 0xE0];
    buf.resize(size - 2, 0x55);
    buf.push(0xFF);
    buf.push(0xD9);
    buf
}

/// Realistic PNG header bytes (89 50 4E 47 0D 0A 1A 0A).
fn fake_png(size: usize) -> Vec<u8> {
    let mut buf = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    buf.resize(size, 0xAA);
    buf
}

// ---------- TxDOT JSON envelope ----------

/// Real TxDOT shape: `application/json` + `{icd_Id, snippet: "<base64 jpeg>"}`.
/// The proxy must decode the base64 and return JPEG bytes.
#[tokio::test]
async fn fetch_txdot_envelope_decodes_json_to_jpeg_bytes() {
    let server = MockServer::start().await;
    let url = format!(
        "{}/its/DistrictIts/GetCctvSnapshotByIcdId",
        server.uri()
    );

    let jpeg = fake_jpeg(2048);
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &jpeg);
    let body = json!({
        "icd_Id": "LP-1 @ Gault Rd",
        "snippet": format!("data:image/jpeg;base64,{}", b64),
    })
    .to_string();

    Mock::given(method("GET"))
        .and(path("/its/DistrictIts/GetCctvSnapshotByIcdId"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(body),
        )
        .expect(1)
        .mount(&server)
        .await;

    let url_with_query =
        format!("{}?icdId=x&districtCode=AUS", url);
    let bytes = fetch_txdot_envelope_via_proxy(&url_with_query)
        .await
        .expect("TxDOT envelope decodes");
    assert_eq!(bytes, jpeg, "decoded bytes must match the original JPEG");
    // Magic bytes verified inside `parse_txdot_envelope` (already unit-tested)
}

/// TxDOT upstream returning 503 must reject so the caller falls through
/// to the Street View / SVG chain rather than caching a stale frame.
#[tokio::test]
async fn fetch_txdot_envelope_rejects_5xx() {
    let server = MockServer::start().await;
    let url = format!("{}/x", server.uri());

    Mock::given(method("GET"))
        .and(path("/x"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let err = fetch_txdot_envelope_via_proxy(&url)
        .await
        .expect_err("5xx must reject");
    assert!(err.contains("503"), "error must mention status: {err}");
}

/// TxDOT upstream returning malformed JSON (e.g. HTML error page
/// wrapped as `text/html`) must reject — the proxy must never serve
/// a non-image body to the browser.
#[tokio::test]
async fn fetch_txdot_envelope_rejects_malformed_json() {
    let server = MockServer::start().await;
    let url = format!("{}/x", server.uri());

    Mock::given(method("GET"))
        .and(path("/x"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string("not actually json"),
        )
        .mount(&server)
        .await;

    let err = fetch_txdot_envelope_via_proxy(&url)
        .await
        .expect_err("malformed JSON must reject");
    assert!(
        err.contains("parse") || err.contains("JSON"),
        "error must mention parse failure: {err}"
    );
}

// ---------- Non-TxDOT upstream body guard ----------

/// Real upstream JPEG with `Content-Type: image/jpeg` must pass the guard.
#[tokio::test]
async fn fetch_image_passes_real_jpeg_with_image_content_type() {
    let server = MockServer::start().await;
    let url = format!("{}/img.jpg", server.uri());
    let jpeg = fake_jpeg(4096);

    Mock::given(method("GET"))
        .and(path("/img.jpg"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "image/jpeg")
                .set_body_bytes(jpeg.clone()),
        )
        .mount(&server)
        .await;

    let (bytes, ct, src) = fetch_image_via_proxy(&url)
        .await
        .expect("real JPEG passes");
    assert_eq!(bytes, jpeg);
    assert_eq!(ct, "image/jpeg");
    assert_eq!(src, "upstream");
}

/// NSW livetraffic returns 200 + `text/html` "Page not found" when a
/// camera URL goes stale. The proxy must reject — passing through with
/// `image/jpeg` would create the original black-screen bug.
#[tokio::test]
async fn fetch_image_rejects_html_404_page_from_nsw() {
    let server = MockServer::start().await;
    let url = format!("{}/dead.jpeg", server.uri());
    let html = b"<html><h4>Page not found</h4></html>".to_vec();

    Mock::given(method("GET"))
        .and(path("/dead.jpeg"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/html")
                .set_body_bytes(html),
        )
        .mount(&server)
        .await;

    let err = fetch_image_via_proxy(&url)
        .await
        .expect_err("HTML 404 page must reject");
    assert!(
        err.contains("rejected"),
        "error must mention body rejection: {err}"
    );
}

/// Upstream returning 200 + missing Content-Type must reject — never
/// serve an unlabeled payload to the browser.
#[tokio::test]
async fn fetch_image_rejects_missing_content_type() {
    let server = MockServer::start().await;
    let url = format!("{}/x.png", server.uri());
    let png = fake_png(2048);

    // No content-type header set on the response.
    Mock::given(method("GET"))
        .and(path("/x.png"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(png))
        .mount(&server)
        .await;

    let err = fetch_image_via_proxy(&url)
        .await
        .expect_err("missing content-type must reject");
    assert!(err.contains("rejected"), "error: {err}");
}

/// Upstream returning 200 + `image/png` with a body that lacks PNG
/// magic bytes must reject (defense in depth — a misconfigured CDN
/// that lies about content-type must not pass).
#[tokio::test]
async fn fetch_image_rejects_content_type_mismatch() {
    let server = MockServer::start().await;
    let url = format!("{}/x.png", server.uri());
    // JPEG bytes masquerading as PNG.
    let jpeg = fake_jpeg(2048);

    Mock::given(method("GET"))
        .and(path("/x.png"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "image/png")
                .set_body_bytes(jpeg),
        )
        .mount(&server)
        .await;

    let err = fetch_image_via_proxy(&url)
        .await
        .expect_err("content-type/magic mismatch must reject");
    assert!(err.contains("rejected"), "error: {err}");
}

// ---------- parse_txdot_envelope edge cases (re-exported) ----------

/// TxDOT occasionally returns `snippet` as a non-string (number, null,
/// object). The helper must reject with a clear error rather than
/// silently producing empty bytes.
#[test]
fn parse_txdot_envelope_rejects_non_string_snippet() {
    for v in [json!(null), json!(123), json!(["a", "b"]), json!({"nested": "x"})] {
        assert!(
            parse_txdot_envelope(&json!({ "snippet": v })).is_err(),
            "non-string snippet must reject: {v:?}"
        );
    }
}