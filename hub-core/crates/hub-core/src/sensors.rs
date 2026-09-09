//! Sensor clients: SearXNG (federated search, §46) and Crawl4AI (§47).
//! Hub-side limits are enforced here; sensors never write to stores directly.

use serde::{Deserialize, Serialize};

use crate::error::{HubError, Result};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResultItem {
    pub url: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub engine: Option<String>,
    #[serde(default)]
    pub score: Option<f64>,
}

pub async fn search_web(state: &AppState, query: &str, limit: usize) -> Result<Vec<SearchResultItem>> {
    let url = format!("{}/search", state.config.searxng_url);
    let resp = state
        .http
        .get(&url)
        .query(&[("q", query), ("format", "json")])
        .send()
        .await
        .map_err(|e| HubError::sensor(format!("searxng unreachable: {e}")))?;
    if !resp.status().is_success() {
        return Err(HubError::sensor(format!("searxng HTTP {}", resp.status())));
    }
    let body: serde_json::Value = resp.json().await?;
    let mut items = Vec::new();
    if let Some(results) = body.get("results").and_then(|r| r.as_array()) {
        for r in results.iter().take(limit) {
            items.push(SearchResultItem {
                url: r.get("url").and_then(|u| u.as_str()).unwrap_or_default().to_string(),
                title: r.get("title").and_then(|t| t.as_str()).map(String::from),
                content: r.get("content").and_then(|c| c.as_str()).map(String::from),
                engine: r.get("engine").and_then(|e| e.as_str()).map(String::from),
                score: r.get("score").and_then(|s| s.as_f64()),
            });
        }
    }
    Ok(items)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawledPage {
    pub url: String,
    pub title: Option<String>,
    pub markdown: String,
    pub links_count: usize,
    pub metadata: serde_json::Value,
}

/// Crawl a single URL via Crawl4AI. Enforces SP2A limits: 1 page per call,
/// hard size cap on returned content (directive §47).
pub async fn crawl_url(state: &AppState, url: &str) -> Result<CrawledPage> {
    const MAX_CONTENT: usize = 512 * 1024; // 512 KiB per page
    let endpoint = format!("{}/crawl", state.config.crawl4ai_url);
    let payload = serde_json::json!({
        "urls": [url],
        "crawler_config": {
            "type": "CrawlerRunConfig",
            "params": { "cache_mode": "bypass", "word_count_threshold": 5 }
        },
        "browser_config": { "type": "BrowserConfig", "params": { "headless": true } }
    });
    let resp = state
        .http
        .post(&endpoint)
        .bearer_auth(&state.config.crawl4ai_api_token)
        .json(&payload)
        .send()
        .await
        .map_err(|e| HubError::sensor(format!("crawl4ai unreachable: {e}")))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(HubError::sensor(format!("crawl4ai HTTP {status}: {}", &text[..text.len().min(300)])));
    }
    let body: serde_json::Value = resp.json().await?;
    if body.get("success").and_then(|s| s.as_bool()) == Some(false) {
        return Err(HubError::sensor(format!(
            "crawl4ai failure: {}",
            body.get("error_message").and_then(|m| m.as_str()).unwrap_or("unknown")
        )));
    }
    let first = body
        .get("results")
        .and_then(|r| r.as_array())
        .and_then(|a| a.first())
        .cloned()
        .ok_or_else(|| HubError::sensor("crawl4ai returned no results"))?;

    let markdown_raw = first
        .get("markdown")
        .and_then(|m| {
            m.as_str().map(String::from).or_else(|| {
                m.get("raw_markdown").and_then(|r| r.as_str()).map(String::from)
            })
        })
        .unwrap_or_default();
    let markdown: String = markdown_raw.chars().take(MAX_CONTENT).collect();
    let metadata = first.get("metadata").cloned().unwrap_or(serde_json::json!({}));
    let title = metadata
        .get("title")
        .and_then(|t| t.as_str())
        .map(String::from)
        .or_else(|| first.get("title").and_then(|t| t.as_str()).map(String::from));
    let links_count = first
        .get("links")
        .and_then(|l| l.get("internal"))
        .and_then(|i| i.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    Ok(CrawledPage {
        url: first.get("url").and_then(|u| u.as_str()).unwrap_or(url).to_string(),
        title,
        markdown,
        links_count,
        metadata,
    })
}
