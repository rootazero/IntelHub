//! Cost Governor (directive §58–60) — gas model + budget state machine.
//!
//! Post-flight metering writes `cost_records`; pre-flight derives the agent's
//! budget state from today's usage (UTC), cached 30s in Redis. States:
//! GREEN <70% · YELLOW 70–90% · RED 90–100% · KILL ≥100%. Per-agent isolation
//! (an agent runaway must not affect other agents, §60) + global ceilings.

use serde::Serialize;
use uuid::Uuid;

use crate::error::Result;
use crate::state::AppState;
use crate::types::{AgentIdentity, BusEvent};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum State {
    Green,
    Yellow,
    Red,
    Kill,
}

impl State {
    pub fn as_str(&self) -> &'static str {
        match self {
            State::Green => "GREEN",
            State::Yellow => "YELLOW",
            State::Red => "RED",
            State::Kill => "KILL",
        }
    }
    fn from_ratio(r: f64) -> State {
        if r >= 1.0 {
            State::Kill
        } else if r >= 0.9 {
            State::Red
        } else if r >= 0.7 {
            State::Yellow
        } else {
            State::Green
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct BudgetState {
    pub state: State,
    /// Worst ratio across all applicable budgets (0.0–∞).
    pub ratio: f64,
    pub usage: Usage,
    pub limits: Limits,
}

#[derive(Clone, Debug, Default, Serialize, serde::Deserialize)]
pub struct Usage {
    pub tool_calls: f64,
    pub crawl_pages: f64,
    pub embed_tokens: f64,
    pub global_crawl_pages: f64,
    pub global_embed_tokens: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Limits {
    pub tool_calls: f64,
    pub crawl_pages: f64,
    pub embed_tokens: f64,
    pub global_crawl_pages: f64,
    pub global_embed_tokens: f64,
}

/// Record one cost event (post-flight metering, §59).
pub async fn record_cost(
    state: &AppState,
    agent_id: Option<Uuid>,
    task_id: Option<Uuid>,
    kind: &str,
    amount: f64,
    unit: &str,
    detail: serde_json::Value,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO cost_records (cost_id, agent_id, task_id, kind, amount, unit, detail)
         VALUES ($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(Uuid::new_v4())
    .bind(agent_id)
    .bind(task_id)
    .bind(kind)
    .bind(amount)
    .bind(unit)
    .bind(detail)
    .execute(&state.pg)
    .await?;
    Ok(())
}

/// Today's usage (UTC) for one agent + global ceilings. Cached 30s in Redis.
pub async fn usage_today(state: &AppState, agent_id: Uuid) -> Result<Usage> {
    let cache_key = format!("hub:budget_usage:{agent_id}");
    let mut conn = state.redis.clone();
    if let Ok(Some(cached)) = redis::cmd("GET")
        .arg(&cache_key)
        .query_async::<Option<String>>(&mut conn)
        .await
    {
        if let Ok(u) = serde_json::from_str::<Usage>(&cached) {
            return Ok(u);
        }
    }
    let rows: Vec<(String, f64)> = sqlx::query_as(
        "SELECT kind, COALESCE(SUM(amount),0) FROM cost_records
         WHERE agent_id = $1 AND created_at >= date_trunc('day', now() AT TIME ZONE 'UTC')
         GROUP BY kind",
    )
    .bind(agent_id)
    .fetch_all(&state.pg)
    .await?;
    let glob: Vec<(String, f64)> = sqlx::query_as(
        "SELECT kind, COALESCE(SUM(amount),0) FROM cost_records
         WHERE created_at >= date_trunc('day', now() AT TIME ZONE 'UTC')
           AND kind IN ('crawl_page','embedding_tokens')
         GROUP BY kind",
    )
    .fetch_all(&state.pg)
    .await?;
    let mut u = Usage::default();
    for (kind, amt) in &rows {
        match kind.as_str() {
            "tool_call" => u.tool_calls = *amt,
            "crawl_page" => u.crawl_pages = *amt,
            "embedding_tokens" => u.embed_tokens = *amt,
            _ => {}
        }
    }
    for (kind, amt) in &glob {
        match kind.as_str() {
            "crawl_page" => u.global_crawl_pages = *amt,
            "embedding_tokens" => u.global_embed_tokens = *amt,
            _ => {}
        }
    }
    if let Ok(s) = serde_json::to_string(&u) {
        let _: redis::RedisResult<()> = redis::cmd("SET")
            .arg(&cache_key)
            .arg(s)
            .arg("EX")
            .arg(30)
            .query_async(&mut conn)
            .await;
    }
    Ok(u)
}

/// Current budget state for an agent (with transition detection + side effects).
pub async fn budget_state(state: &AppState, agent: &AgentIdentity) -> Result<BudgetState> {
    let cfg = &state.config;
    let usage = usage_today(state, agent.agent_id).await.unwrap_or_default();
    let limits = Limits {
        tool_calls: cfg.budget_agent_tool_calls,
        crawl_pages: cfg.budget_agent_crawl_pages,
        embed_tokens: cfg.budget_agent_embed_tokens,
        global_crawl_pages: cfg.budget_global_crawl_pages,
        global_embed_tokens: cfg.budget_global_embed_tokens,
    };
    let ratio = [
        usage.tool_calls / limits.tool_calls,
        usage.crawl_pages / limits.crawl_pages,
        usage.embed_tokens / limits.embed_tokens,
        usage.global_crawl_pages / limits.global_crawl_pages,
        usage.global_embed_tokens / limits.global_embed_tokens,
    ]
    .into_iter()
    .fold(0.0_f64, f64::max);
    let cur = State::from_ratio(ratio);

    // Transition detection (previous state persisted in Redis).
    let prev_key = format!("hub:budget_state:{}", agent.agent_id);
    let mut conn = state.redis.clone();
    let prev: Option<String> = redis::cmd("GET")
        .arg(&prev_key)
        .query_async(&mut conn)
        .await
        .ok()
        .flatten();
    let prev_state = prev.as_deref().unwrap_or("GREEN");
    if prev_state != cur.as_str() {
        let _: redis::RedisResult<()> = redis::cmd("SET")
            .arg(&prev_key)
            .arg(cur.as_str())
            .query_async(&mut conn)
            .await;
        on_transition(state, agent, prev_state, cur, ratio).await;
    }

    Ok(BudgetState { state: cur, ratio, usage, limits })
}

/// Side effects of a budget state transition: bus event + alert; KILL also
/// terminates the agent's running tasks (§60: terminate task, save results).
async fn on_transition(
    state: &AppState,
    agent: &AgentIdentity,
    prev: &str,
    cur: State,
    ratio: f64,
) {
    if cur == State::Green {
        return; // recovery is quiet
    }
    let ev_type = if cur >= State::Red { "BUDGET_EXCEEDED" } else { "BUDGET_WARNING" };
    crate::events::publish(
        state,
        BusEvent::new(
            ev_type,
            &format!("agent:{}", agent.name),
            serde_json::json!({
                "from": prev, "to": cur.as_str(), "ratio": (ratio * 100.0).round() / 100.0
            }),
        ),
    )
    .await;
    let _ = crate::alerts::raise(
        state,
        crate::alerts::NewAlert {
            severity: if cur >= State::Red { "critical" } else { "warning" },
            source: "budget",
            title: &format!("Agent {} budget → {} ({:.0}%)", agent.name, cur.as_str(), ratio * 100.0),
            body: Some(&format!("previous state {prev}")),
            task_id: None,
            investigation_id: None,
            entity_name: None,
            evidence_id: None,
            recommended_action: Some(match cur {
                State::Yellow => "Reduce exploration depth and concurrency",
                State::Red => "Stop new search branches; process existing evidence only",
                _ => "Terminate run; review runaway behavior before raising budget",
            }),
            dedupe_key: Some(&format!("budget:{}:{}", agent.name, cur.as_str())),
        },
    )
    .await;
    if cur == State::Kill {
        // §60: terminate the runaway agent's running tasks (results stay saved).
        let _ = sqlx::query(
            "UPDATE tasks SET status='terminated', finished_at=now(),
             detail = detail || '{\"reason\":\"budget KILL\"}'::jsonb
             WHERE status IN ('running','pending') AND created_by = $1",
        )
        .bind(format!("agent:{}", agent.name))
        .execute(&state.pg)
        .await;
    }
}

/// YELLOW-state throttle check for the embedding worker: auto (non-forced)
/// embedding jobs pause while any budget is ≥70% (§60: reduce exploration).
pub async fn embeddings_paused(state: &AppState) -> bool {
    let cfg = &state.config;
    let glob: Option<(f64, f64)> = sqlx::query_as(
        "SELECT
           COALESCE(SUM(amount) FILTER (WHERE kind='crawl_page'),0),
           COALESCE(SUM(amount) FILTER (WHERE kind='embedding_tokens'),0)
         FROM cost_records WHERE created_at >= date_trunc('day', now() AT TIME ZONE 'UTC')",
    )
    .fetch_optional(&state.pg)
    .await
    .ok()
    .flatten();
    match glob {
        Some((crawl, embed)) => {
            crawl / cfg.budget_global_crawl_pages >= 0.7
                || embed / cfg.budget_global_embed_tokens >= 0.7
        }
        None => false,
    }
}
