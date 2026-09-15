use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::Tier;

/// Unified Evidence Event envelope (directive §50) — the single entry
/// shape for all content before any AI analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceEvent {
    pub event_id: Uuid,
    pub source: String,
    pub url: String,
    pub retrieved_at: DateTime<Utc>,
    #[serde(default)]
    pub published_at: Option<DateTime<Utc>>,
    pub content_hash: String,
    pub content: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
    #[serde(default)]
    pub provenance: serde_json::Value,
}

/// Event-bus envelope for hub.events (directive §63 subset).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusEvent {
    pub event_id: Uuid,
    pub event_type: String,
    pub ts: DateTime<Utc>,
    pub actor: String,
    #[serde(default)]
    pub investigation_id: Option<Uuid>,
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub payload: serde_json::Value,
}

impl BusEvent {
    pub fn new(event_type: &str, actor: &str, payload: serde_json::Value) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            event_type: event_type.to_string(),
            ts: Utc::now(),
            actor: actor.to_string(),
            investigation_id: None,
            trace_id: None,
            payload,
        }
    }
    pub fn with_investigation(mut self, id: Uuid) -> Self {
        self.investigation_id = Some(id);
        self
    }
    pub fn with_trace(mut self, trace: &str) -> Self {
        self.trace_id = Some(trace.to_string());
        self
    }
}

/// Authenticated agent identity, inserted into request extensions by the
/// auth middleware and read by MCP tool handlers via RequestContext.
#[derive(Debug, Clone)]
pub struct AgentIdentity {
    pub agent_id: Uuid,
    pub name: String,
    pub key_id: Uuid,
    /// True when the request also carried a valid X-Admin-Token (SP2B Level 3).
    /// The token itself is never logged; only this derived flag is audited.
    pub admin: bool,
    /// Caller tier resolved from `agents.tier` at authentication time.
    /// Synthetic identities (console aggregates, MCP fallback) default to Free.
    pub tier: Tier,
    /// First 12 chars of the presented API key. Used solely for `audit_log`
    /// attribution (`api_key_prefix`); never the full secret.
    pub key_prefix: String,
}

/// Trace context for a single MCP/HTTP request.
#[derive(Debug, Clone)]
pub struct RequestTrace {
    pub trace_id: String,
    pub request_id: String,
}

impl RequestTrace {
    pub fn new() -> Self {
        Self {
            trace_id: Uuid::new_v4().to_string(),
            request_id: Uuid::new_v4().to_string(),
        }
    }
}

impl Default for RequestTrace {
    fn default() -> Self {
        Self::new()
    }
}
