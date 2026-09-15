//! OSV (Open Source Vulnerabilities) — Google's CC-BY open database of
//! vulnerabilities in OSS packages across 10+ ecosystems (PyPI, npm,
//! Go, crates.io, Maven, RubyGems, NuGet, Packagist, Hex, OSS-Fuzz).
//! Complements NVD (general CVE) and CISA KEV (exploited-only):
//! OSV is ecosystem-specific and tracks affected version ranges with
//! ecosystem-specific resolution that doesn't translate cleanly into
//! CPE/NVD format. Free, no key, no rate limit.
//!
//! Built-in watchlist covers packages common to OSINT infrastructure;
//! override with HUB_OSV_WATCH as `ecosystem:package` pairs. Each
//! sweep queries each package, filters vulns modified within the
//! last 24h, emits a Signal per new/updated entry. Anchored at OSV
//! HQ (Mountain View, CA) — distinct from NVD HQ and CISA HQ so all
//! three cyber sources are visually separable on the Radar map.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::{HubError, Result};

use super::super::{Ctx, Signal, Source};

const API_BASE: &str = "https://api.osv.dev/v1";
const OSV_HQ: (f64, f64) = (37.4220, -122.0841); // Mountain View, CA
const LOOKBACK_HOURS: i64 = 24;
const MAX_PER_PACKAGE: usize = 5;

/// Built-in watchlist: packages common to OSINT/data infrastructure.
/// Each entry is (ecosystem, package_name). Override with
/// HUB_OSV_WATCH as comma-separated "ecosystem:package" pairs.
const DEFAULT_WATCH: &[(&str, &str)] = &[
    ("PyPI", "django"),
    ("PyPI", "cryptography"),
    ("PyPI", "pillow"),
    ("PyPI", "requests"),
    ("npm", "lodash"),
    ("npm", "axios"),
    ("Go", "github.com/golang/oauth2"),
    ("crates.io", "rustls"),
    ("crates.io", "tokio"),
    ("crates.io", "reqwest"),
    ("Maven", "org.apache.logging.log4j:log4j-core"),
    ("RubyGems", "rails"),
];

pub struct Osv;

impl Source for Osv {
    fn name(&self) -> &'static str {
        "osv"
    }
    fn interval(&self) -> Duration {
        // 6h cadence — most packages see weekly-level vulns; this
        // catches them within a working day.
        Duration::from_secs(6 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            // Build watchlist: env override takes precedence over built-in.
            let watch: Vec<(String, String)> = if !ctx.config.monitor_osv_watch.is_empty() {
                ctx.config
                    .monitor_osv_watch
                    .iter()
                    .filter_map(|s| {
                        let mut parts = s.splitn(2, ':');
                        let eco = parts.next()?.trim().to_string();
                        let pkg = parts.next()?.trim().to_string();
                        if eco.is_empty() || pkg.is_empty() {
                            None
                        } else {
                            Some((eco, pkg))
                        }
                    })
                    .collect()
            } else {
                DEFAULT_WATCH
                    .iter()
                    .map(|(e, p)| (e.to_string(), p.to_string()))
                    .collect()
            };

            if watch.is_empty() {
                return Ok(Vec::new());
            }

            let now = chrono::Utc::now();
            let cutoff = now - chrono::Duration::hours(LOOKBACK_HOURS);

            let mut out = Vec::new();
            for (ecosystem, package) in watch {
                // OSV's query endpoint returns ALL vulns for a package;
                // we filter client-side by modified timestamp.
                let req_body = serde_json::json!({
                    "package": { "name": package, "ecosystem": ecosystem }
                });
                let resp = match ctx
                    .http
                    .post(format!("{API_BASE}/query"))
                    .json(&req_body)
                    .send()
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        // Per-package fetch error: skip this package,
                        // don't fail the whole sweep. Reduces noise
                        // from one missing/typo'd package.
                        tracing::warn!(target: "monitor::osv",
                            "skip {ecosystem}/{package}: {e}");
                        continue;
                    }
                };
                if !resp.status().is_success() {
                    tracing::warn!(target: "monitor::osv",
                        "skip {ecosystem}/{package}: HTTP {}", resp.status());
                    continue;
                }
                let j: serde_json::Value = resp.json().await?;
                let vulns = j
                    .get("vulns")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();

                for v in vulns.into_iter().take(MAX_PER_PACKAGE) {
                    let vuln_id = v.get("id").and_then(|x| x.as_str()).unwrap_or("");
                    if vuln_id.is_empty() {
                        continue;
                    }

                    let modified_str = v.get("modified").and_then(|x| x.as_str()).unwrap_or("");
                    let modified_at = chrono::DateTime::parse_from_rfc3339(modified_str)
                        .ok()
                        .map(|d| d.with_timezone(&chrono::Utc));

                    // Delta filter — only emit vulns modified in the
                    // lookback window. Vulns older than 24h are skipped.
                    let Some(modified_at) = modified_at else { continue };
                    if modified_at < cutoff {
                        continue;
                    }

                    let summary = v
                        .get("summary")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                    let details = v
                        .get("details")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();

                    // OSV severity is heterogeneous — CVSS_V3 vector
                    // string, CVSS_V4 vector, or domain-specific label
                    // (Ubuntu "high", GHSA "Moderate"). Parse the vector
                    // for Impact/Exploitability markers as a coarse
                    // severity hint; default to routine.
                    let (sev_type, sev_score) = v
                        .get("severity")
                        .and_then(|s| s.as_array())
                        .and_then(|arr| arr.first())
                        .map(|s| {
                            (
                                s.get("type")
                                    .and_then(|x| x.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                s.get("score")
                                    .and_then(|x| x.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                            )
                        })
                        .unwrap_or_default();

                    let severity = infer_severity(&sev_type, &sev_score, &details);

                    let title = {
                        let s: String = summary.chars().take(120).collect();
                        format!("{vuln_id} ({ecosystem}/{package}): {s}")
                    };

                    let aliases: Vec<String> = v
                        .get("aliases")
                        .and_then(|a| a.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|x| x.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();

                    let references: Vec<String> = v
                        .get("references")
                        .and_then(|r| r.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|r| {
                                    r.get("url")
                                        .and_then(|u| u.as_str())
                                        .map(String::from)
                                })
                                .collect()
                        })
                        .unwrap_or_default();

                    out.push(
                        Signal::new(
                            "cyber",
                            title,
                            OSV_HQ.0,
                            OSV_HQ.1,
                            format!("osv:{ecosystem}:{package}:{vuln_id}"),
                        )
                        .severity(match severity {
                            "flash" => "flash",
                            "priority" => "priority",
                            _ => "routine",
                        })
                        .occurred(modified_at)
                        .payload(serde_json::json!({
                            "vuln_id": vuln_id,
                            "ecosystem": ecosystem,
                            "package": package,
                            "aliases": aliases,
                            "severity_type": sev_type,
                            "severity_score": sev_score,
                            "summary": summary,
                            "details": details.chars().take(2000).collect::<String>(),
                            "references": references,
                        })),
                    );
                }
            }
            if out.is_empty() {
                // Sweep completed but no deltas — return empty (not Err)
                // so the health cell stays "ok", not "degraded".
                return Ok(Vec::new());
            }
            Ok(out)
        }
        .boxed()
    }
}

/// Coarse severity inference: parse CVSS vector for Impact/Exploitability
/// markers, or use domain-label hints (Ubuntu/GHSA "Critical"/"High").
/// Returns one of "flash" | "priority" | "routine".
fn infer_severity(sev_type: &str, sev_score: &str, details: &str) -> &'static str {
    let s_upper = sev_score.to_uppercase();
    if s_upper.contains("CRITICAL") || s_upper.contains("9.") || s_upper.contains("10.") {
        return "flash";
    }
    if s_upper.contains("HIGH") || s_upper.contains("7.") || s_upper.contains("8.") {
        return "priority";
    }
    // CVSS vector: high impact markers indicate high severity
    // regardless of computed base score (e.g., ADJACENT network + HIGH CIA)
    if sev_type.starts_with("CVSS") {
        let d_upper = details.to_uppercase();
        if d_upper.contains("REMOTE CODE EXECUTION")
            || d_upper.contains("ARBITRARY CODE")
            || d_upper.contains("AUTHENTICATION BYPASS")
        {
            return "flash";
        }
    }
    "routine"
}

#[cfg(test)]
mod tests {
    use super::infer_severity;

    #[test]
    fn cvss_vector_high_marks() {
        // CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H → 9.8 critical
        let s = "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H";
        assert_eq!(infer_severity("CVSS_V3", s, ""), "flash");
    }

    #[test]
    fn ghsa_label() {
        assert_eq!(infer_severity("GHSA", "Critical", ""), "flash");
        assert_eq!(infer_severity("GHSA", "High", ""), "priority");
    }

    #[test]
    fn default_routine() {
        assert_eq!(infer_severity("", "", "low-impact info leak"), "routine");
    }
}