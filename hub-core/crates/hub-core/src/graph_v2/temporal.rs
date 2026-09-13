//! Bi-temporal merge policy (spec §2).
//! valid_from (NULL = unknown start), valid_until (NULL = still true or unknown end).

use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Overlap {
    /// Same window — should update in place, not insert new.
    SameWindow,
    /// Overlap but not identical — possible conflict; default policy = treat as supersede (new wins).
    Overlap,
    /// Adjacent: a ends exactly when b starts (or vice versa) — REPLACES edge.
    Adjacent,
    /// Fully disjoint — independent relationship.
    Disjoint,
}

pub fn classify_overlap(
    a_from: Option<DateTime<Utc>>,
    a_until: Option<DateTime<Utc>>,
    b_from: Option<DateTime<Utc>>,
    b_until: Option<DateTime<Utc>>,
) -> Overlap {
    // SameWindow: both ranges identical (Some/Some == Some/Some, or all NULL).
    if a_from == b_from && a_until == b_until {
        return Overlap::SameWindow;
    }
    // Adjacent: a.until == b.from (or vice versa).
    if let (Some(au), Some(bf)) = (a_until, b_from) {
        if au == bf {
            return Overlap::Adjacent;
        }
    }
    if let (Some(af), Some(bu)) = (a_from, b_until) {
        if af == bu {
            return Overlap::Adjacent;
        }
    }
    // Disjoint: one ends before the other begins (in either direction).
    let disjoint = || -> bool {
        // a ends at-or-before b starts: a.until <= b.from
        if let (Some(au), Some(bf)) = (a_until, b_from) {
            if au <= bf {
                return true;
            }
        }
        // b ends at-or-before a starts: b.until <= a.from
        if let (Some(bu), Some(af)) = (b_until, a_from) {
            if bu <= af {
                return true;
            }
        }
        false
    };
    if disjoint() {
        Overlap::Disjoint
    } else {
        Overlap::Overlap
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn t(s: &str) -> DateTime<Utc> {
        // Fixed format `YYYY-MM-DD HH:MM:SS` interpreted as UTC. Using
        // NaiveDateTime::parse_from_str + and_utc avoids the deprecated
        // chrono::TimeZone::datetime_from_str path.
        chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
            .unwrap()
            .and_utc()
    }

    #[test]
    fn identical_is_same_window() {
        assert_eq!(
            classify_overlap(
                Some(t("2025-01-01 00:00:00")),
                Some(t("2025-12-31 00:00:00")),
                Some(t("2025-01-01 00:00:00")),
                Some(t("2025-12-31 00:00:00"))
            ),
            Overlap::SameWindow
        );
    }

    #[test]
    fn nulls_same_window() {
        assert_eq!(classify_overlap(None, None, None, None), Overlap::SameWindow);
    }

    #[test]
    fn adjacent_detected() {
        assert_eq!(
            classify_overlap(
                Some(t("2025-01-01 00:00:00")),
                Some(t("2025-06-01 00:00:00")),
                Some(t("2025-06-01 00:00:00")),
                None
            ),
            Overlap::Adjacent
        );
    }

    #[test]
    fn overlap_detected() {
        assert_eq!(
            classify_overlap(
                Some(t("2025-01-01 00:00:00")),
                Some(t("2025-08-01 00:00:00")),
                Some(t("2025-06-01 00:00:00")),
                Some(t("2025-12-31 00:00:00"))
            ),
            Overlap::Overlap
        );
    }

    #[test]
    fn disjoint_detected() {
        assert_eq!(
            classify_overlap(
                Some(t("2024-01-01 00:00:00")),
                Some(t("2024-06-01 00:00:00")),
                Some(t("2025-01-01 00:00:00")),
                Some(t("2025-06-01 00:00:00"))
            ),
            Overlap::Disjoint
        );
    }
}
