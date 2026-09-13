//! Integration tests for A: query cache key generation. Pure — no IO.
//! Lives in /tests/ as its own binary (sidesteps pre-existing unit-test
//! compile failures in hub_core::monitor::sources).

use hub_core::cache::{cache_key, CacheMode};

/// Design invariant: same args → same key (deterministic cache hits).
#[test]
fn cache_key_stable_for_same_args() {
    let k1 = cache_key(CacheMode::Hybrid, "BRICS", 10, None);
    let k2 = cache_key(CacheMode::Hybrid, "BRICS", 10, None);
    assert_eq!(k1, k2);
}

/// Different query must produce different key (no false cache hits).
#[test]
fn cache_key_differs_on_query() {
    let k1 = cache_key(CacheMode::Hybrid, "BRICS", 10, None);
    let k2 = cache_key(CacheMode::Hybrid, "yields", 10, None);
    assert_ne!(k1, k2);
}

/// Different limit must produce different key (cache is per-result-set).
#[test]
fn cache_key_differs_on_limit() {
    let k1 = cache_key(CacheMode::Hybrid, "BRICS", 10, None);
    let k2 = cache_key(CacheMode::Hybrid, "BRICS", 20, None);
    assert_ne!(k1, k2);
}

/// url_contains filter must participate (semantically a different query).
#[test]
fn cache_key_differs_on_url_filter() {
    let k1 = cache_key(CacheMode::Hybrid, "BRICS", 10, None);
    let k2 = cache_key(CacheMode::Hybrid, "BRICS", 10, Some("wikipedia.org"));
    assert_ne!(k1, k2);
}

/// Same url_contains string must collapse to the same key.
#[test]
fn cache_key_stable_for_same_url_filter() {
    let k1 = cache_key(CacheMode::Hybrid, "BRICS", 10, Some("wikipedia.org"));
    let k2 = cache_key(CacheMode::Hybrid, "BRICS", 10, Some("wikipedia.org"));
    assert_eq!(k1, k2);
}

/// Different search mode must produce different key — hybrid and semantic
/// results are NOT interchangeable (different scoring, different items).
#[test]
fn cache_key_differs_on_mode() {
    let h = cache_key(CacheMode::Hybrid, "BRICS", 10, None);
    let s = cache_key(CacheMode::Semantic, "BRICS", 10, None);
    let k = cache_key(CacheMode::Keyword, "BRICS", 10, None);
    assert_ne!(h, s);
    assert_ne!(h, k);
    assert_ne!(s, k);
}

/// Whitespace normalization: trailing/leading spaces and case folded —
/// otherwise "BRICS" and " brics " would miss the cache.
#[test]
fn cache_key_normalizes_whitespace_and_case() {
    let k1 = cache_key(CacheMode::Hybrid, "BRICS de-dollarization", 10, None);
    let k2 = cache_key(CacheMode::Hybrid, "  brics   DE-DOLLARIZATION  ", 10, None);
    assert_eq!(k1, k2, "whitespace + case differences must collapse");
}

/// Key format: short, hex, prefixed so we can grep + delete by prefix.
#[test]
fn cache_key_format() {
    let k = cache_key(CacheMode::Hybrid, "BRICS", 10, None);
    assert!(k.starts_with("hub:qcache:"), "key prefix: {k}");
    let suffix = k.trim_start_matches("hub:qcache:");
    assert_eq!(suffix.len(), 32, "hex sha256/2 prefix (16 bytes = 32 hex): {suffix}");
}

/// Mode-specific prefixes avoid cross-mode pollution in Redis.
#[test]
fn cache_key_includes_mode_in_prefix() {
    let h = cache_key(CacheMode::Hybrid, "BRICS", 10, None);
    let s = cache_key(CacheMode::Semantic, "BRICS", 10, None);
    // First 16 hex chars after the prefix should differ
    let h_mid = &h[12..28];
    let s_mid = &s[12..28];
    assert_ne!(h_mid, s_mid, "mode bytes must be part of hash input");
}
