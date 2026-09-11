//! Title-based kind classifier shared by the GDELT and RSS sources.
//!
//! Both pull broad news/conflict firehoses whose titles mix hard conflict,
//! politics, and markets. A shared keyword pass peels `political` /
//! `financial` / `conflict` out of the default `news` bucket so the Radar
//! taxonomy carries information instead of one 300-event "news" smear.
//! First hit wins; check order is conflict → political → financial.

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

/// Classify a headline. `None` = no signal → caller keeps its default kind.
pub(crate) fn classify_title(title: &str) -> Option<&'static str> {
    let t = title.to_lowercase();
    for (kind, words) in [
        ("conflict", CONFLICT),
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
    }
}
