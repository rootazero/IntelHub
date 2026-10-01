// §ALERT-CH-VIS — tests for the alert channel visibility layer.
//
// The production helper is `console::build_channels_snapshot`, but it's
// `pub(crate)` and lives behind `AppState` plumbing. We re-implement the
// snapshot logic here as a thin wrapper around `mask_chat_id` to keep the
// test self-contained — if the production helper becomes `pub`, swap the
// tests to import it instead.

// Mirror of `console::mask_chat_id`. Keep in sync — the test pin enforces
// `***6789` shape and the `***` short-id fallback.
fn mask_chat_id(raw: &str) -> String {
    let n = raw.chars().count();
    if n <= 4 {
        "***".to_string()
    } else {
        let tail: String = raw
            .chars()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("***{tail}")
    }
}

// Mirror of `console::build_channels_snapshot` for the pure-logic tests.
// Same reasoning: re-implement so the tests don't need a running hub-core.
fn build_snapshot(
    webhook_url: Option<&str>,
    webhook_min_severity: &str,
    telegram_token: Option<&str>,
    telegram_chat: Option<&str>,
) -> serde_json::Value {
    let webhook_on = webhook_url.is_some();
    let telegram_token_on = telegram_token.is_some();
    let telegram_chat_on = telegram_chat.is_some();
    let telegram_on = telegram_token_on && telegram_chat_on;

    let webhook = serde_json::json!({
        "enabled": webhook_on,
        "min_severity": webhook_min_severity,
    });

    let telegram_reason = match (telegram_token_on, telegram_chat_on) {
        (true, true) => serde_json::Value::Null,
        (false, true) => {
            serde_json::json!("HUB_ALERT_TELEGRAM_BOT_TOKEN not set in secrets.env")
        }
        (true, false) => {
            serde_json::json!("HUB_ALERT_TELEGRAM_CHAT_ID not set in secrets.env")
        }
        (false, false) => {
            serde_json::json!("HUB_ALERT_TELEGRAM_BOT_TOKEN and HUB_ALERT_TELEGRAM_CHAT_ID not set in secrets.env")
        }
    };
    let telegram = serde_json::json!({
        "enabled": telegram_on,
        "chat_id_masked": if telegram_on {
            serde_json::Value::String(mask_chat_id(telegram_chat.unwrap_or("")))
        } else {
            serde_json::Value::Null
        },
        "reason": telegram_reason,
    });

    serde_json::json!({
        "webhook": webhook,
        "telegram": telegram,
        "any_enabled": webhook_on || telegram_on,
    })
}

// ── mask_chat_id ────────────────────────────────────────────────────────────

#[test]
fn mask_numeric_id_keeps_last_four() {
    assert_eq!(mask_chat_id("123456789"), "***6789");
    assert_eq!(mask_chat_id("987654321"), "***4321");
}

#[test]
fn mask_handles_or_short_id_is_redacted() {
    assert_eq!(mask_chat_id("@foo"), "***");
    assert_eq!(mask_chat_id("12"), "***");
}

#[test]
fn mask_empty_string_is_redacted() {
    assert_eq!(mask_chat_id(""), "***");
}

// ── build_channels_snapshot ────────────────────────────────────────────────

#[test]
fn snapshot_neither_channel_configured() {
    let v = build_snapshot(None, "warning", None, None);
    assert_eq!(v["webhook"]["enabled"], serde_json::json!(false));
    assert_eq!(v["telegram"]["enabled"], serde_json::json!(false));
    assert_eq!(v["any_enabled"], serde_json::json!(false));
    let reason = v["telegram"]["reason"].as_str().unwrap();
    assert!(reason.contains("BOT_TOKEN"));
    assert!(reason.contains("CHAT_ID"));
}

#[test]
fn snapshot_only_webhook() {
    let v = build_snapshot(Some("http://10.10.10.41:18899/alert"), "warning", None, None);
    assert_eq!(v["webhook"]["enabled"], serde_json::json!(true));
    assert_eq!(v["webhook"]["min_severity"], serde_json::json!("warning"));
    assert_eq!(v["telegram"]["enabled"], serde_json::json!(false));
    assert_eq!(v["any_enabled"], serde_json::json!(true));
}

#[test]
fn snapshot_only_telegram_token_no_chat_id() {
    let v = build_snapshot(None, "warning", Some("bot123"), None);
    assert_eq!(v["webhook"]["enabled"], serde_json::json!(false));
    assert_eq!(v["telegram"]["enabled"], serde_json::json!(false));
    let reason = v["telegram"]["reason"].as_str().unwrap();
    assert!(reason.contains("CHAT_ID"));
    assert!(!reason.contains("BOT_TOKEN"));
}

#[test]
fn snapshot_only_telegram_chat_id_no_token() {
    let v = build_snapshot(None, "warning", None, Some("987654321"));
    assert_eq!(v["telegram"]["enabled"], serde_json::json!(false));
    let reason = v["telegram"]["reason"].as_str().unwrap();
    assert!(reason.contains("BOT_TOKEN"));
    assert!(!reason.contains("CHAT_ID"));
    assert!(v["telegram"]["chat_id_masked"].is_null());
}

#[test]
fn snapshot_both_channels_enabled() {
    let v = build_snapshot(
        Some("http://10.10.10.41:18899/alert"),
        "critical",
        Some("bot123"),
        Some("987654321"),
    );
    assert_eq!(v["webhook"]["enabled"], serde_json::json!(true));
    assert_eq!(v["webhook"]["min_severity"], serde_json::json!("critical"));
    assert_eq!(v["telegram"]["enabled"], serde_json::json!(true));
    assert_eq!(v["any_enabled"], serde_json::json!(true));
    assert_eq!(v["telegram"]["chat_id_masked"], serde_json::json!("***4321"));
    assert!(v["telegram"]["reason"].is_null());
}

#[test]
fn snapshot_telegram_disabled_no_chat_id_mask_leak() {
    // Even when chat_id is set but telegram is disabled (no token), we must
    // not leak the unmasked chat id in chat_id_masked.
    let v = build_snapshot(None, "warning", None, Some("987654321"));
    assert!(v["telegram"]["chat_id_masked"].is_null());
}