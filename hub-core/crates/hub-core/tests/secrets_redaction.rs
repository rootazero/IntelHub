//! Secrets-redaction integration tests.
//!
//! For any paid-source event, the response payload MUST NOT contain the
//! substring of the upstream API key. Free agents in particular must be
//! unable to extract the key via repeated queries.

use reqwest::Client;
use serde_json::Value;
use std::collections::HashSet;

const HUB: &str = "http://10.10.10.45:8800";

fn free_key() -> String {
    std::env::var("TIER_TEST_FREE_KEY").expect("TIER_TEST_FREE_KEY env")
}

fn known_key_substrings() -> HashSet<String> {
    // First 16 chars of each live key. Loaded from a file the operator
    // pre-populates before running tests.
    let path = std::env::var("TIER_TEST_KEY_SUBSTR")
        .unwrap_or_else(|_| "/tmp/tier-key-substrs.txt".to_string());
    std::fs::read_to_string(&path)
        .expect("read substrs file")
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

#[tokio::test]
async fn no_key_in_responses() {
    let client = Client::new();
    let key = free_key();
    let substrs = known_key_substrings();

    // Hit all geo_events-touching surfaces as a free agent.
    let urls = [
        format!("{HUB}/api/v1/events/recent"),
        format!("{HUB}/api/v1/events/recent?source=monitor:x"),
        format!("{HUB}/api/v1/events/recent?source=monitor:finintel"),
        format!("{HUB}/api/v1/overview"),
        format!("{HUB}/api/v1/search/unified?q=test"),
    ];

    for url in &urls {
        let resp = client
            .get(url)
            .header("Authorization", format!("Bearer {key}"))
            .send()
            .await
            .unwrap();
        let body = resp.text().await.unwrap();

        for sub in &substrs {
            assert!(
                !body.contains(sub),
                "response from {url} contains key substring {sub:?} — leak"
            );
        }
    }
}

#[tokio::test]
async fn no_key_in_payload_json() {
    let client = Client::new();
    let key = free_key();
    let substrs = known_key_substrings();

    // Trigger a search that returns documents with payloads.
    let resp: Value = client
        .get(format!("{HUB}/api/v1/search/unified?q=test"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let body = serde_json::to_string(&resp).unwrap();

    for sub in &substrs {
        assert!(
            !body.contains(sub),
            "search response payload contains key substring {sub:?}"
        );
    }
}

#[tokio::test]
async fn no_key_in_logs() {
    // This test reads the hub-core log file. Assumes journald path or
    // syslog. The operator must run this test on the same VM as hub-core
    // (or share the log file via the same path).
    let log_path = std::env::var("HUB_CORE_LOG")
        .unwrap_or_else(|_| "/home/zou/IntelHub/core/hub.log".to_string());
    let substrs = known_key_substrings();

    // Trigger a few requests first so any logging happens.
    let client = Client::new();
    let key = free_key();
    for _ in 0..3 {
        let _ = client
            .get(format!("{HUB}/api/v1/events/recent"))
            .header("Authorization", format!("Bearer {key}"))
            .send()
            .await;
    }
    // Give logs 200ms to flush.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Read recent log tail (10K bytes).
    let mut cmd = std::process::Command::new("tail");
    cmd.arg("-c").arg("10000").arg(&log_path);
    let output = cmd.output().expect("tail log file");
    let log_text = String::from_utf8_lossy(&output.stdout);

    for sub in &substrs {
        assert!(
            !log_text.contains(sub),
            "log file contains key substring {sub:?}"
        );
    }
}
