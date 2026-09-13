//! Boot-time Neo4j schema enforcement. Idempotent: uses IF NOT EXISTS for every
//! constraint and index. Reads Cypher from scripts/neo4j_init_cypher.txt.

use neo4rs::Graph;
use crate::error::HubError;

/// Run all statements in scripts/neo4j_init_cypher.txt against `g`.
/// Statements are split on `;` and run individually. Failures bubble up.
pub async fn ensure_neo4j_schema(g: &Graph) -> Result<(), HubError> {
    let cypher = include_str!("../../../scripts/neo4j_init_cypher.txt");
    for stmt in split_statements(cypher) {
        let trimmed = stmt.trim();
        if trimmed.is_empty() { continue; }
        g.run(neo4rs::query(trimmed)).await.map_err(|e| {
            HubError::Internal(format!("neo4j schema init failed: {e}; stmt={trimmed}"))
        })?;
    }
    Ok(())
}

fn split_statements(s: &str) -> impl Iterator<Item = &str> {
    s.split(';')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_semicolon() {
        let s = "CREATE CONSTRAINT a IF NOT EXISTS FOR (n:L) REQUIRE n.id IS UNIQUE;\n\nCREATE INDEX b IF NOT EXISTS FOR (n:L) ON (n.name);\n";
        // Trailing newline after the last `;` yields a third empty chunk; filter it.
        let v: Vec<&str> = split_statements(s).filter(|s| !s.trim().is_empty()).collect();
        assert_eq!(v.len(), 2);
        assert!(v[0].contains("CONSTRAINT"));
        assert!(v[1].contains("INDEX"));
    }

    #[test]
    fn skips_empty_statements() {
        let s = ";;A;;";
        let v: Vec<&str> = split_statements(s).filter(|s| !s.trim().is_empty()).collect();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0], "A");
    }
}
