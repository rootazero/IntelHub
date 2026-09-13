//! Policy Engine (directive §61) — registry-driven single enforcement point.
//!
//! Every tool is statically declared with a risk level and cost class.
//! The MCP/REST wrapper calls `preflight()` BEFORE executing any tool and
//! `postflight()` after — no tool can bypass governance. Unknown tool names
//! default to L1/Free (rmcp only routes registered tools, so this is a
//! defensive fallback, not a reachable path for unvetted tools).

use crate::error::{HubError, Result};
use crate::state::AppState;
use crate::types::AgentIdentity;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RiskLevel {
    /// §61 Level 1: allowed automatically.
    L1Auto,
    /// §61 Level 2: requires schema validation (handled by the tool itself).
    L2Validated,
    /// §61 Level 3: default DENY; requires a valid X-Admin-Token on the request.
    L3AdminOnly,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CostClass {
    Free,
    Search,
    Crawl,
    Write,
    Admin,
}

pub struct ToolPolicy {
    pub level: RiskLevel,
    pub cost: CostClass,
}

/// The static tool registry — the single source of truth for governance.
pub fn policy_for(tool: &str) -> ToolPolicy {
    use CostClass::*;
    use RiskLevel::*;
    let (level, cost) = match tool {
        // SP2A read/search plane (L1)
        "search_web" => (L1Auto, Search),
        "get_document" | "get_evidence" | "keyword_search" | "hybrid_search"
        | "semantic_search" | "query_entity" | "query_relationship" | "find_path"
        | "list_investigations" | "get_task_status" | "get_system_health" => (L1Auto, Free),
        "create_investigation" | "update_investigation" | "create_finding" => (L1Auto, Write),
        // SP2A crawl plane (L2: URL scheme validation in tool)
        "crawl_url" | "fetch_document" => (L2Validated, Crawl),
        // SP2B graph writes (L2: typed intent schema validation in graphw.rs)
        "create_entity" | "create_claim" | "create_relationship" => (L2Validated, Write),
        // SP9 graph reads (L1: read-only Cypher/SQL templates, no writes)
        "search_entity" | "get_entity" | "get_entity_timeline" | "get_neighbors"
        | "find_relationship_changes" | "find_supporting_claims"
        | "find_contradicting_claims" | "query_investigation_graph"
        | "list_evidence_for_entity" => (L1Auto, Free),
        // SP2B alerts + budgets (L1)
        "list_alerts" | "acknowledge_alert" | "mute_alert" | "get_budget_status" => {
            (L1Auto, Free)
        }
        // SP2B lifecycle (L3: default DENY, admin token required)
        "run_component_action" => (L3AdminOnly, Admin),
        // SP6B monitor finance plane
        "signal_query" => (L1Auto, Free),
        "financials_fetch" => (L2Validated, Crawl), // pay-per-request upstream, 30d-cached
        "watchlist_manage" => (L2Validated, Write),
        _ => (L1Auto, Free),
    };
    ToolPolicy { level, cost }
}

/// Tools still allowed when an agent is in RED (stop expanding new branches,
/// keep processing existing evidence, §60).
const RED_ALLOWED: &[&str] = &[
    "get_document", "get_evidence", "keyword_search", "query_entity",
    "query_relationship", "find_path", "list_investigations", "create_investigation",
    "update_investigation", "create_finding", "get_task_status", "get_system_health",
    "list_alerts", "acknowledge_alert", "mute_alert", "get_budget_status",
    "create_entity", "create_claim", "create_relationship", "signal_query",
];

/// Tools still allowed in KILL (terminate task, save current results, §60).
const KILL_ALLOWED: &[&str] = &[
    "get_document", "get_evidence", "keyword_search", "list_investigations",
    "get_task_status", "get_system_health", "list_alerts", "acknowledge_alert",
    "mute_alert", "get_budget_status",
];

/// Full pre-flight: Level-3 admin check → budget state check.
/// Returns the current budget state so callers can embed it in responses.
pub async fn preflight(
    state: &AppState,
    agent: &AgentIdentity,
    tool: &str,
) -> Result<crate::cost::BudgetState> {
    let pol = policy_for(tool);

    // ── Level 3 gate (directive §61: default DENY, explicit authorization) ──
    if pol.level == RiskLevel::L3AdminOnly && !agent.admin {
        let _ = crate::store::record_audit(
            &state.pg,
            &format!("agent:{}", agent.name),
            "policy_denied",
            Some("tool"),
            Some(tool),
            "denied",
            serde_json::json!({
                "level": 3,
                "why": "Level 3 action requires X-Admin-Token",
                "source": "policy_engine",
            }),
        )
        .await;
        let _ = crate::alerts::raise(
            state,
            crate::alerts::NewAlert {
                severity: "warning",
                source: "security",
                title: &format!("Level 3 denied: {} attempted {}", agent.name, tool),
                body: None,
                task_id: None,
                investigation_id: None,
                entity_name: None,
                evidence_id: None,
                recommended_action: Some("Review whether this agent should hold an admin token"),
                dedupe_key: Some(&format!("security:l3deny:{}:{}", agent.name, tool)),
            },
        )
        .await;
        return Err(HubError::policy_denied(
            "Level 3 action requires a valid X-Admin-Token",
        ));
    }

    // ── Budget gate (directive §60) ──
    let bs = crate::cost::budget_state(state, agent).await?;
    match bs.state {
        crate::cost::State::Kill if !KILL_ALLOWED.contains(&tool) => {
            Err(HubError::BudgetDenied {
                state: "KILL".into(),
                reason: format!(
                    "agent budget exhausted — tool '{}' denied; save results and stop",
                    tool
                ),
            })
        }
        crate::cost::State::Red if !RED_ALLOWED.contains(&tool) => {
            Err(HubError::BudgetDenied {
                state: "RED".into(),
                reason: format!(
                    "budget ≥90% — new search branches stopped; tool '{}' denied",
                    tool
                ),
            })
        }
        _ => Ok(bs),
    }
}
