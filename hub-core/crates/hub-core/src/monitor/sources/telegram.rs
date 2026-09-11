//! Telegram public-channel watch (SP8-C social plane, keyless).
//!
//! `https://t.me/s/<channel>` is the public web preview — no bot API, no
//! auth — serving the ~20 newest messages as server-rendered HTML. We parse
//! the widget blocks (data-post id, <time datetime>, message text), geotag
//! the text, and skip non-mappable posts. Watchlist overridable via
//! TG_WATCH="channel|kind,channel". These are official conflict-zone
//! channels → default kind political / news (media), never conflict by
//! default: a statement ABOUT a strike is not a strike (classifier may
//! still upgrade on hard keywords).

use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use futures::FutureExt;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};
use super::{rss::geotag, textclass::{classify_title, static_kind}};

/// (channel, default kind) — all verified via t.me/s on 2026-09-11.
pub const DEFAULT_WATCH: &[(&str, &str)] = &[
    ("V_Zelenskiy_official", "political"),
    ("Ukraine_MFA", "political"),
    ("idfofficial", "political"),
    ("medvedev_telegram", "political"),
    ("nexta_live", "news"),
    ("KyivIndependent_official", "news"),
];

pub struct TelegramWatch;

fn watchlist(cfg: &[String]) -> Vec<(String, String)> {
    if cfg.is_empty() {
        return DEFAULT_WATCH.iter().map(|(c, k)| (c.to_string(), k.to_string())).collect();
    }
    cfg.iter()
        .map(|e| {
            let mut parts = e.splitn(2, '|');
            let c = parts.next().unwrap_or("").trim().to_string();
            let k = parts.next().unwrap_or("political").trim().to_string();
            (c, k)
        })
        .filter(|(c, _)| !c.is_empty())
        .collect()
}

impl Source for TelegramWatch {
    fn name(&self) -> &'static str {
        "telegram-watch"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(1800)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let mut out: Vec<Signal> = Vec::new();
            for (chan, defkind) in watchlist(&ctx.config.tg_watch) {
                let url = format!("https://t.me/s/{chan}");
                match ctx.http.get(&url).send().await {
                    Ok(r) if r.status().is_success() => {
                        let html = r.text().await.unwrap_or_default();
                        out.extend(parse_channel(&html, &chan, &defkind));
                    }
                    Ok(r) => tracing::warn!(channel = %chan, status = %r.status(), "tg http"),
                    Err(e) => tracing::warn!(channel = %chan, error = %e, "tg fetch"),
                }
            }
            Ok(out)
        }
        .boxed()
    }
}

/// One parsed message: (post id, datetime, plain text).
pub struct TgMsg {
    pub id: String,
    pub ts: Option<DateTime<Utc>>,
    pub text: String,
}

/// t.me/s HTML → messages. Hand-rolled marker parsing (no extra deps):
/// every message lives in a `data-post="<chan>/<id>"` block containing a
/// `<time datetime="...">` and a `.tgme_widget_message_text` div.
pub fn parse_messages(html: &str) -> Vec<TgMsg> {
    let mut out = Vec::new();
    for block in html.split("data-post=\"").skip(1) {
        let Some(id) = block.split('\"').next() else { continue };
        let ts = block
            .split("<time datetime=\"")
            .nth(1)
            .and_then(|s| s.split('\"').next())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.with_timezone(&Utc));
        let text = block
            .split("tgme_widget_message_text")
            .nth(1)
            .and_then(|s| s.split_once('>').map(|(_, rest)| rest))
            .and_then(|s| s.split("</div>").next())
            .map(strip_tags)
            .unwrap_or_default();
        if !text.is_empty() {
            out.push(TgMsg { id: id.to_string(), ts, text });
        }
    }
    out
}

fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                if in_tag {
                    out.push(' ');
                }
                in_tag = false;
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    // cheap entity cleanup for the common cases
    out.replace("&amp;", "&")
        .replace("&#39;", "'")
        .replace("&quot;", "\"")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Channel page → geo signals (mappable-only).
pub fn parse_channel(html: &str, chan: &str, defkind: &str) -> Vec<Signal> {
    let mut out = Vec::new();
    for msg in parse_messages(html) {
        if msg.text.len() < 20 {
            continue;
        }
        let Some((lat, lon)) = geotag(&msg.text) else { continue };
        let msg_id = msg.id.rsplit('/').next().unwrap_or("x");
        let mut sig = Signal::new(
            classify_title(&msg.text).unwrap_or(static_kind(defkind)),
            msg.text.chars().take(120).collect::<String>(),
            lat,
            lon,
            format!("tg:{}:{}", chan, msg_id),
        )
        .severity("info");
        if let Some(t) = msg.ts {
            sig = sig.occurred(t);
        }
        out.push(sig.payload(serde_json::json!({
            "channel": chan,
            "url": format!("https://t.me/{chan}/{msg_id}"),
        })));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_widget_blocks() {
        let html = r#"
        <div class="tgme_widget_message text_not_supported_wrap js-widget_message" data-post="idfofficial/1234">
          <div class="tgme_widget_message_text js-message_text" dir="auto">IDF struck a missile site in Syria overnight.<br/>Details to follow.</div>
          <a class="tgme_widget_message_date" href="https://t.me/idfofficial/1234"><time datetime="2026-09-11T06:30:00+00:00">06:30</time></a>
        </div>
        <div class="tgme_widget_message" data-post="idfofficial/1233">
          <div class="tgme_widget_message_text">ok</div>
          <time datetime="2026-09-11T05:00:00+00:00">05:00</time>
        </div>"#;
        let msgs = parse_messages(html);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].id, "idfofficial/1234");
        assert!(msgs[0].text.contains("missile site in Syria"));
        let sigs = parse_channel(html, "idfofficial", "political");
        assert_eq!(sigs.len(), 1, "short message skipped");
        assert_eq!(sigs[0].kind, "conflict"); // "missile" keyword wins
        assert_eq!(sigs[0].external_id, "tg:idfofficial:1234");
    }

    #[test]
    fn strip_tags_handles_breaks() {
        assert_eq!(strip_tags("hello<br/>world &amp; peace"), "hello world & peace");
    }
}
