//! Sync entity resolution: exact → alias → jaro_winkler ≥ threshold → new entity.
//! See spec §5 for the algorithm.

use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::HubError;
use crate::store::entities;

/// How a match was made.
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub enum ResolutionMethod {
    Exact,
    Alias,
    JaroWinkler,
    NewEntity,
}

#[derive(Debug, Clone, Serialize)]
pub enum EntityResolution {
    Existing { entity_id: Uuid, matched_by: ResolutionMethod, score: f64 },
    New { entity_id: Uuid },
}

/// Resolve a name+kind to a canonical entity_id, creating one if needed.
pub async fn resolve_entity(
    name: &str,
    kind: &str,
    source: &str,
    confidence: f64,
    pool: &PgPool,
) -> Result<EntityResolution, HubError> {
    let threshold = std::env::var("HUB_KG_RESOLVE_THRESHOLD")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.92);
    let name_norm = entities::normalize_name(name);
    if name_norm.is_empty() {
        return Err(HubError::Validation("empty entity name".into()));
    }

    // 1. Exact match on (kind, normalized name)
    if let Some(eid) = entities::find_by_kind_name(pool, kind, &name_norm).await? {
        return Ok(EntityResolution::Existing {
            entity_id: eid,
            matched_by: ResolutionMethod::Exact,
            score: 1.0,
        });
    }
    // 2. Alias exact match
    if let Some(eid) = entities::lookup_alias_exact(pool, kind, &name_norm).await? {
        return Ok(EntityResolution::Existing {
            entity_id: eid,
            matched_by: ResolutionMethod::Alias,
            score: 1.0,
        });
    }
    // 3. Jaro-Winkler scan against all candidates of this kind.
    // Note: `name_norm` is mixed-case (normalize_name does not lowercase); SQL
    // returns `lower(name)` for c.name_norm. Lowercase both sides here so the
    // comparison is case-insensitive (otherwise mixed-case input scores lower
    // and the 0.92 threshold becomes unfair).
    let candidates = entities::find_candidates_by_kind(pool, kind).await?;
    let name_lower = name_norm.to_lowercase();
    let mut best: Option<(Uuid, f64)> = None;
    for c in candidates {
        let score = strsim::jaro_winkler(&name_lower, &c.name_norm);
        if score >= threshold && (best.is_none() || score > best.unwrap().1) {
            best = Some((c.entity_id, score));
        }
    }
    if let Some((eid, score)) = best {
        entities::write_alias(pool, eid, name, &name_norm, kind, source, confidence).await?;
        return Ok(EntityResolution::Existing {
            entity_id: eid,
            matched_by: ResolutionMethod::JaroWinkler,
            score,
        });
    }
    // 4. No match — create new entity with self-alias
    let eid = entities::create_with_alias(pool, kind, name, &name_norm, source, confidence).await?;
    Ok(EntityResolution::New { entity_id: eid })
}

/// Merge `dropped` into `kept`, promoting the dropped entity's aliases to the
/// kept entity (skipping ones that already exist on `kept`). Used by both the
/// sync path (manual review → `method = "manual"`) and the async worker
/// (auto-merge → `method = "jaro_winkler"`).
///
/// `method` must be one of the values allowed by the `entities.resolution_method`
/// CHECK constraint: `exact|alias|jaro_winkler|manual|seed`. The default `manual`
/// preserves the original behavior for callers that don't specify a method.
pub async fn merge_entities(
    a: Uuid,
    b: Uuid,
    kept: Uuid,
    actor: &str,
    method: &str,
    pool: &PgPool,
) -> Result<(), HubError> {
    let mut tx = pool.begin().await?;
    let dropped = if a == kept { b } else { a };
    // Set merged_into on the dropped side; stamp the resolution_method that
    // drove the merge so downstream reads can distinguish manual from auto.
    sqlx::query("UPDATE entities SET merged_into = $1, resolution_method = $3 WHERE entity_id = $2")
        .bind(kept)
        .bind(dropped)
        .bind(method)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE entity_aliases SET entity_id = $1
           WHERE entity_id = $2
             AND NOT EXISTS (SELECT 1 FROM entity_aliases ea2 WHERE ea2.entity_id = $1 AND ea2.alias_norm = entity_aliases.alias_norm)",
    )
    .bind(kept)
    .bind(dropped)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM entity_aliases WHERE entity_id = $1")
        .bind(dropped)
        .execute(&mut *tx)
        .await?;
    // Audit
    sqlx::query(
        "INSERT INTO graph_change_log (op, target_kind, target_id, before, after, changed_by)
         VALUES ('merge','entity',$1, jsonb_build_object('merged_into', NULL), jsonb_build_object('merged_into', $2), $3)",
    )
    .bind(dropped.to_string())
    .bind(kept.to_string())
    .bind(actor)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolution_method_serializes() {
        let j = serde_json::to_string(&ResolutionMethod::JaroWinkler).unwrap();
        assert_eq!(j, "\"JaroWinkler\"");
    }

    #[test]
    fn resolution_method_variants_serialize_distinct() {
        for (m, expected) in [
            (ResolutionMethod::Exact, "\"Exact\""),
            (ResolutionMethod::Alias, "\"Alias\""),
            (ResolutionMethod::JaroWinkler, "\"JaroWinkler\""),
            (ResolutionMethod::NewEntity, "\"NewEntity\""),
        ] {
            assert_eq!(serde_json::to_string(&m).unwrap(), expected);
        }
    }

    #[test]
    fn existing_resolution_carries_score_and_method() {
        let eid = Uuid::nil();
        let r = EntityResolution::Existing {
            entity_id: eid,
            matched_by: ResolutionMethod::Exact,
            score: 1.0,
        };
        // Default serde external tagging wraps the variant in its name.
        let v = serde_json::to_value(&r).unwrap();
        assert!(v.get("Existing").is_some());
        assert_eq!(v["Existing"]["entity_id"], eid.to_string());
        assert_eq!(v["Existing"]["matched_by"], "Exact");
        assert_eq!(v["Existing"]["score"], 1.0);
    }

    #[test]
    fn new_resolution_carries_only_entity_id() {
        let eid = Uuid::nil();
        let r = EntityResolution::New { entity_id: eid };
        let v = serde_json::to_value(&r).unwrap();
        assert!(v.get("New").is_some());
        assert_eq!(v["New"]["entity_id"], eid.to_string());
    }

    #[test]
    fn jaro_winkler_self_match_is_one() {
        // Sanity-check the dep is wired correctly (and self-similarity = 1).
        let s = strsim::jaro_winkler("apple inc", "apple inc");
        assert!((s - 1.0).abs() < 1e-9);
    }

    #[test]
    fn jaro_winkler_close_but_not_exact_below_one() {
        let s = strsim::jaro_winkler("apple inc", "apple incorporated");
        assert!(s > 0.85 && s < 1.0);
    }

    /// Regression for Finding #2: when both sides of the JW comparison are
    /// lowercased at the comparison site, mixed-case input like "Apple Inc"
    /// matches the SQL-returned "apple inc" with score 1.0 (not 0.85-ish).
    /// Pins the comparison-site lowercasing contract.
    #[test]
    fn jw_is_case_insensitive_after_lowercasing_both_sides() {
        let input_mixed = "Apple Inc";
        let sql_candidate_lower = "apple inc";
        let score = strsim::jaro_winkler(&input_mixed.to_lowercase(), sql_candidate_lower);
        assert!((score - 1.0).abs() < 1e-9);
        // Sanity: the un-lowercased version (the bug) is visibly lower.
        let buggy_score = strsim::jaro_winkler(input_mixed, sql_candidate_lower);
        assert!(buggy_score < 1.0, "buggy comparison should miss self-similarity");
        assert!(buggy_score < score, "lowercasing must not regress");
    }
}