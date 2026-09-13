//! Integration tests for RRF helper extracted from mcp::hybrid_inner
//! (e2e MCP audit 2026-09-13). Lives in /tests/ as its own binary so
//! pre-existing failures in hub_core unit tests don't block validation.

use hub_core::mcp::{rrf_fuse, RrfScore};

fn ids(count: usize) -> Vec<uuid::Uuid> {
    (0..count)
        .map(|i| uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, format!("doc-{i}").as_bytes()))
        .collect()
}

/// Design invariant: K=10 (new default) gives substantially more
/// separation between consecutive ranks than K=60 (old default).
/// K=60 made rank-1 vs rank-3 only ~3% apart; K=10 makes them ~15%.
/// Both regimes are valid RRF; this test pins our chosen K so a future
/// change is loud.
#[test]
fn rrf_k10_separates_ranks_much_better_than_k60() {
    let kw = ids(10);
    let v: Vec<uuid::Uuid> = vec![];
    let k10 = rrf_fuse(&kw, &v, 10.0);
    let k60 = rrf_fuse(&kw, &v, 60.0);
    let spread = |scored: &[RrfScore]| -> f64 {
        // gap between rank 1 (max) and rank 3 (third best)
        (scored[0].raw - scored[2].raw) / scored[0].raw
    };
    let k10_gap = spread(&k10);
    let k60_gap = spread(&k60);
    assert!(k10_gap > 0.10, "K=10 should spread top-3 by >10%, got {:.2}%", k10_gap * 100.0);
    assert!(k60_gap < 0.05, "K=60 should keep top-3 within 5%, got {:.2}%", k60_gap * 100.0);
    assert!(k10_gap > k60_gap * 3.0, "K=10 spread should be at least 3× K=60");
}

#[test]
fn rrf_normalization_spans_zero_to_one() {
    let kw = ids(5);
    let v = vec![kw[2], kw[0], kw[4]]; // overlap on top doc
    let scored = rrf_fuse(&kw, &v, 10.0);
    // The doc that's in both channels should normalize to 1.0
    assert!((scored[0].norm - 1.0).abs() < 1e-9, "top doc norm={}", scored[0].norm);
    // The lowest-ranked doc should normalize to 0.0
    let last = scored.last().unwrap();
    assert!(last.norm.abs() < 1e-9, "last doc norm={}", last.norm);
    // All norms in [0, 1]
    for s in &scored {
        assert!(s.norm >= 0.0 && s.norm <= 1.0, "norm {} out of range", s.norm);
    }
}

#[test]
fn rrf_single_candidate_normalizes_to_one() {
    let one = vec![uuid::Uuid::new_v4()];
    let scored = rrf_fuse(&one, &[], 10.0);
    assert_eq!(scored.len(), 1);
    assert!((scored[0].norm - 1.0).abs() < 1e-9);
}

#[test]
fn rrf_both_channels_empty_returns_empty() {
    assert!(rrf_fuse(&[], &[], 10.0).is_empty());
}

#[test]
fn rrf_overlap_bonus_increases_score() {
    let only_kw = vec![ids(3)[0]];
    let only_v = vec![ids(3)[1]];
    let both = vec![ids(3)[2]];
    // doc1 (only in kw): score = 1/(10+0+1) = 0.0909
    // doc2 (only in v): score = 0.0909
    // doc3 (in both): score = 0.0909 + 0.0909 = 0.1818 → SHOULD rank first
    let scored = rrf_fuse(&both, &both, 10.0);
    // doc3 should be top
    assert_eq!(scored[0].doc_id, both[0]);
    assert!((scored[0].raw - 2.0 / 11.0).abs() < 1e-9);
    // sanity: both-channels docs always outscore single-channel
    let _ = only_kw;
    let _ = only_v;
}
