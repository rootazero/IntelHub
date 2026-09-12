//! Title-based kind classifier shared by the GDELT and RSS sources.
//!
//! Both pull broad news/conflict firehoses whose titles mix hard conflict,
//! politics, and markets. A shared keyword pass peels `political` /
//! `financial` / `conflict` out of the default `news` bucket so the Radar
//! taxonomy carries information instead of one 300-event "news" smear.
//! First hit wins; check order is conflict → climate → political → financial.
//!
//! Climate boundary (user decision 2026-09-11): acute physical events stay
//! `disaster` (NOAA/ReliefWeb set their own kinds, never this classifier);
//! this set only catches climate-SYSTEM signals — trends, policy, science —
//! never hazard words (flood/storm/wildfire stay disaster).

const CONFLICT: &[&str] = &[
    "airstrike", "air strike", "missile", "drone strike", "shelling", "ceasefire",
    "hostage", "clashes", "bombing", "insurgent", "militia", "artillery", "ambush",
    "offensive", "troops", "killed", "airstrikes",
];
const POLITICAL: &[&str] = &[
    "election", "protest", "coup", "referendum", "parliament", "impeach",
    "sanction", "tariff", "diplomat", "treaty", "summit", "unrest",
    "demonstration", "president", "prime minister", "vote",
];
const FINANCIAL: &[&str] = &[
    "stock", "markets", "inflation", "interest rate", "bond", "recession",
    "earnings", "ipo", "bitcoin", "crypto", "oil price", "bailout",
    "wall street", "federal reserve", "central bank", "treasury",
];
// Climate-system terms only: no hazard words (those are `disaster` semantics).
const CLIMATE: &[&str] = &[
    "climate change", "global warming", "climate crisis", "climate summit",
    "climate policy", "carbon emission", "carbon tax", "carbon border",
    "net zero", "net-zero", "greenhouse gas", "ipcc", "paris agreement",
    "decarboni", "emissions deal", "emissions cut", "hottest month",
    "hottest year", "hottest day", "cop30", "cop31", "unfccc",
];

/// Config-string kind → &'static (Signal::new requires static; unknown → news).
pub(crate) fn static_kind(k: &str) -> &'static str {
    match k {
        "conflict" => "conflict",
        "political" => "political",
        "financial" => "financial",
        "health" => "health",
        "military" => "military",
        "climate" => "climate",
        "cyber" => "cyber",
        _ => "news",
    }
}

/// Classify a headline. `None` = no signal → caller keeps its default kind.
pub(crate) fn classify_title(title: &str) -> Option<&'static str> {
    let t = title.to_lowercase();
    for (kind, words) in [
        ("conflict", CONFLICT),
        ("climate", CLIMATE),
        ("political", POLITICAL),
        ("financial", FINANCIAL),
    ] {
        if words.iter().any(|w| t.contains(w)) {
            return Some(kind);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peels_categories() {
        assert_eq!(classify_title("Missile strike hits port"), Some("conflict"));
        assert_eq!(classify_title("Protesters storm parliament after election"), Some("political"));
        assert_eq!(classify_title("Stocks fall as inflation rises"), Some("financial"));
        assert_eq!(classify_title("New vaccine trial results"), None);
        // conflict wins over political when both match
        assert_eq!(classify_title("Troops deployed after election unrest"), Some("conflict"));
        // climate = system/policy/science, and wins over political/financial
        assert_eq!(classify_title("June was the hottest month on record"), Some("climate"));
        assert_eq!(classify_title("EU carbon border tax takes effect"), Some("climate"));
        assert_eq!(classify_title("Summit deadlock over emissions cut pledges"), Some("climate"));
        // hazard words alone do NOT flip to climate (disaster semantics)
        assert_eq!(classify_title("Hurricane makes landfall"), None);
    }
}
