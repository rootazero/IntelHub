//! Integration tests for the T8star rerank response parser.
//! Pure — no IO. Lives in /tests/ as its own binary to sidestep the
//! pre-existing unit-test compile failures in hub_core::monitor::sources.

use hub_core::rerank::parse_rerank_response;
use uuid::Uuid;

fn ids(count: usize) -> Vec<Uuid> {
    (0..count)
        .map(|i| Uuid::new_v5(&Uuid::NAMESPACE_OID, format!("rerank-doc-{i}").as_bytes()))
        .collect()
}

fn candidates(texts: &[&str]) -> Vec<(Uuid, String)> {
    let u = ids(texts.len());
    u.into_iter().zip(texts.iter().map(|s| s.to_string())).collect()
}

/// Real T8star response shape (probe 2026-09-13): each hit carries an
/// `index` back into the original documents array + a `relevance_score`.
/// Higher score = more relevant. Parser must respect both.
#[test]
fn parse_preserves_rerank_order_and_maps_index_to_candidate_id() {
    let body = r#"{
      "results": [
        {"index": 0, "relevance_score": 0.003524, "document": {"text": "BRICS New Development Bank"}},
        {"index": 2, "relevance_score": 0.001748, "document": {"text": "mBridge CBDC settlement"}},
        {"index": 3, "relevance_score": 0.000333, "document": {"text": "yuan SWIFT share"}},
        {"index": 1, "relevance_score": 0.000016, "document": {"text": "NBA Knicks finals"}}
      ]
    }"#;
    let cands = candidates(&["NDB", "NBA", "mBridge", "yuan"]);
    let scored = parse_rerank_response(body, &cands).expect("must parse");

    // Top result is NDB (index 0 → first candidate id), in order
    assert_eq!(scored.len(), 4);
    assert_eq!(scored[0].0, cands[0].0, "NDB should rank first");
    assert_eq!(scored[1].0, cands[2].0, "mBridge should rank second");
    assert_eq!(scored[2].0, cands[3].0, "yuan should rank third");
    assert_eq!(scored[3].0, cands[1].0, "NBA noise should rank last");

    // Scores must be sorted descending
    for w in scored.windows(2) {
        assert!(w[0].1 >= w[1].1, "scores not descending: {} vs {}", w[0].1, w[1].1);
    }
}

/// Malformed JSON → error (caller falls back to pre-rerank ranking).
#[test]
fn parse_malformed_json_returns_err() {
    let cands = candidates(&["a", "b"]);
    assert!(parse_rerank_response("not json", &cands).is_err());
    assert!(parse_rerank_response("{", &cands).is_err());
}

/// `results` field missing → error.
#[test]
fn parse_missing_results_field_returns_err() {
    let body = r#"{"data": []}"#;
    let cands = candidates(&["a"]);
    assert!(parse_rerank_response(body, &cands).is_err());
}

/// Empty `results` array → empty ranked list (graceful, not error).
#[test]
fn parse_empty_results_returns_empty() {
    let body = r#"{"results": []}"#;
    let cands = candidates(&["a", "b"]);
    let scored = parse_rerank_response(body, &cands).unwrap();
    assert!(scored.is_empty());
}

/// Out-of-range `index` (model returned something we didn't send) → skip,
/// don't crash. Should never happen with a healthy model but the failure
/// mode must degrade gracefully.
#[test]
fn parse_out_of_range_index_skipped() {
    let body = r#"{
      "results": [
        {"index": 0, "relevance_score": 0.5, "document": {"text": "a"}},
        {"index": 99, "relevance_score": 0.9, "document": {"text": "ghost"}}
      ]
    }"#;
    let cands = candidates(&["a", "b"]);
    let scored = parse_rerank_response(body, &cands).unwrap();
    // Only index=0 maps to a real candidate; ghost is silently dropped
    assert_eq!(scored.len(), 1);
    assert_eq!(scored[0].0, cands[0].0);
}

/// Duplicate index in response (degenerate model output) → keep first.
#[test]
fn parse_duplicate_index_keeps_first() {
    let body = r#"{
      "results": [
        {"index": 0, "relevance_score": 0.5, "document": {"text": "a"}},
        {"index": 0, "relevance_score": 0.9, "document": {"text": "a-dup"}}
      ]
    }"#;
    let cands = candidates(&["a", "b"]);
    let scored = parse_rerank_response(body, &cands).unwrap();
    assert_eq!(scored.len(), 1, "duplicate index should not double-count");
}
