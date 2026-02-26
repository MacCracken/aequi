use chrono::NaiveDate;

use crate::util::levenshtein_distance;

#[derive(Debug, Clone)]
pub struct MatchableTransaction {
    pub id: i64,
    pub date: NaiveDate,
    pub description: String,
    pub amount_cents: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MatchType {
    Exact,
    DateAndAmount,
    Fuzzy { score: f32 },
    None,
}

#[derive(Debug, Clone)]
pub struct MatchResult {
    pub imported_tx_id: i64,
    pub matched_tx_id: Option<i64>,
    pub match_type: MatchType,
    pub confidence: f32,
    pub difference_cents: i64,
}

pub struct AutoMatchEngine {
    pub date_window_days: i32,
    pub fuzzy_threshold: f32,
    pub amount_tolerance_cents: i64,
}

impl Default for AutoMatchEngine {
    fn default() -> Self {
        Self {
            date_window_days: 3,
            fuzzy_threshold: 0.7,
            amount_tolerance_cents: 1,
        }
    }
}

impl AutoMatchEngine {
    pub fn new(date_window_days: i32, fuzzy_threshold: f32, amount_tolerance_cents: i64) -> Self {
        Self {
            date_window_days,
            fuzzy_threshold,
            amount_tolerance_cents,
        }
    }

    pub fn find_matches(
        &self,
        imported: &[MatchableTransaction],
        existing: &[MatchableTransaction],
    ) -> Vec<MatchResult> {
        imported
            .iter()
            .map(|imp| self.find_best_match(imp, existing))
            .collect()
    }

    fn find_best_match(
        &self,
        imp: &MatchableTransaction,
        existing: &[MatchableTransaction],
    ) -> MatchResult {
        let best = existing
            .iter()
            .filter_map(|exp| self.score_pair(imp, exp))
            .max_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));

        match best {
            Some((tx_id, match_type, confidence, diff)) => MatchResult {
                imported_tx_id: imp.id,
                matched_tx_id: Some(tx_id),
                match_type,
                confidence,
                difference_cents: diff,
            },
            None => MatchResult {
                imported_tx_id: imp.id,
                matched_tx_id: None,
                match_type: MatchType::None,
                confidence: 0.0,
                difference_cents: 0,
            },
        }
    }

    /// Returns `Some((tx_id, match_type, confidence, diff_cents))` if the pair
    /// clears the amount tolerance and fuzzy threshold, else `None`.
    fn score_pair(
        &self,
        imp: &MatchableTransaction,
        exp: &MatchableTransaction,
    ) -> Option<(i64, MatchType, f32, i64)> {
        let diff_cents = (imp.amount_cents - exp.amount_cents).abs();
        if diff_cents > self.amount_tolerance_cents {
            return None;
        }

        let date_diff = (imp.date - exp.date).num_days().unsigned_abs() as i32;
        if date_diff > self.date_window_days {
            return None;
        }

        // Perfect hit — no further computation needed.
        if date_diff == 0 && diff_cents == 0 {
            return Some((exp.id, MatchType::Exact, 1.0, 0));
        }

        let date_score = 1.0 - (date_diff as f32 / (self.date_window_days + 1) as f32);
        let desc_score = description_similarity(&imp.description, &exp.description);
        let confidence = (date_score + desc_score) / 2.0;

        if confidence >= self.fuzzy_threshold {
            let match_type = if date_diff == 0 {
                MatchType::DateAndAmount
            } else {
                MatchType::Fuzzy { score: confidence }
            };
            Some((exp.id, match_type, confidence, diff_cents))
        } else {
            None
        }
    }
}

/// Normalises a description to lowercase alphanumeric words and computes
/// Levenshtein similarity in the range [0.0, 1.0].
fn description_similarity(s1: &str, s2: &str) -> f32 {
    let a = normalize(s1);
    let b = normalize(s2);

    if a == b {
        return 1.0;
    }

    let max_len = a.len().max(b.len());
    if max_len == 0 {
        return 1.0;
    }

    1.0 - (levenshtein_distance(&a, &b) as f32 / max_len as f32)
}

fn normalize(s: &str) -> String {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Detect likely duplicate transactions within a slice.
/// Returns pairs of IDs that are within `window_days` of each other,
/// share the same amount, and have description similarity >= `threshold`.
pub fn find_duplicates(
    transactions: &[MatchableTransaction],
    window_days: i32,
    threshold: f32,
) -> Vec<(i64, i64)> {
    let mut duplicates = Vec::new();

    for i in 0..transactions.len() {
        for j in (i + 1)..transactions.len() {
            let t1 = &transactions[i];
            let t2 = &transactions[j];

            if t1.amount_cents != t2.amount_cents {
                continue;
            }
            let date_diff = (t1.date - t2.date).num_days().unsigned_abs() as i32;
            if date_diff > window_days {
                continue;
            }
            if description_similarity(&t1.description, &t2.description) >= threshold {
                duplicates.push((t1.id, t2.id));
            }
        }
    }

    duplicates
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tx(id: i64, date: (i32, u32, u32), desc: &str, amount: i64) -> MatchableTransaction {
        MatchableTransaction {
            id,
            date: NaiveDate::from_ymd_opt(date.0, date.1, date.2).unwrap(),
            description: desc.to_string(),
            amount_cents: amount,
        }
    }

    #[test]
    fn exact_match_same_date_and_amount() {
        let engine = AutoMatchEngine::default();
        let imported = vec![tx(1, (2024, 1, 15), "AMAZON MARKETPLACE", 4999)];
        let existing = vec![tx(100, (2024, 1, 15), "AMAZON MARKETPLACE", 4999)];
        let results = engine.find_matches(&imported, &existing);
        assert_eq!(results[0].matched_tx_id, Some(100));
        assert_eq!(results[0].match_type, MatchType::Exact);
        assert_eq!(results[0].confidence, 1.0);
    }

    #[test]
    fn no_match_different_amount() {
        let engine = AutoMatchEngine::default();
        let imported = vec![tx(1, (2024, 1, 15), "TEST", 1000)];
        let existing = vec![tx(100, (2024, 1, 15), "TEST", 2000)];
        let results = engine.find_matches(&imported, &existing);
        assert_eq!(results[0].matched_tx_id, None);
        assert_eq!(results[0].match_type, MatchType::None);
    }

    #[test]
    fn no_match_outside_date_window() {
        let engine = AutoMatchEngine::default(); // window = 3 days
        let imported = vec![tx(1, (2024, 1, 15), "TEST", 1000)];
        let existing = vec![tx(100, (2024, 1, 20), "TEST", 1000)]; // 5 days away
        let results = engine.find_matches(&imported, &existing);
        assert_eq!(results[0].matched_tx_id, None);
    }

    #[test]
    fn fuzzy_match_within_date_window() {
        let engine = AutoMatchEngine::default();
        let imported = vec![tx(1, (2024, 1, 15), "AMAZON MARKETPLACE", 4999)];
        let existing = vec![tx(100, (2024, 1, 17), "AMAZON MARKETPLACE", 4999)]; // 2 days off
        let results = engine.find_matches(&imported, &existing);
        assert_eq!(results[0].matched_tx_id, Some(100));
        assert!(matches!(results[0].match_type, MatchType::Fuzzy { .. }));
    }

    #[test]
    fn picks_best_of_multiple_candidates() {
        let engine = AutoMatchEngine::default();
        let imported = vec![tx(1, (2024, 1, 15), "AMAZON", 1000)];
        let existing = vec![
            tx(100, (2024, 1, 15), "TOTALLY DIFFERENT", 1000), // same date/amount, bad desc
            tx(101, (2024, 1, 15), "AMAZON", 1000),            // perfect
        ];
        let results = engine.find_matches(&imported, &existing);
        assert_eq!(results[0].matched_tx_id, Some(101));
    }

    #[test]
    fn find_duplicates_detects_identical() {
        let txs = vec![
            tx(1, (2024, 1, 15), "STARBUCKS", 500),
            tx(2, (2024, 1, 15), "STARBUCKS", 500),
            tx(3, (2024, 1, 20), "WHOLE FOODS", 3000),
        ];
        let dups = find_duplicates(&txs, 3, 0.9);
        assert_eq!(dups.len(), 1);
        assert_eq!(dups[0], (1, 2));
    }

    #[test]
    fn find_duplicates_ignores_different_amounts() {
        let txs = vec![
            tx(1, (2024, 1, 15), "STARBUCKS", 500),
            tx(2, (2024, 1, 15), "STARBUCKS", 600),
        ];
        assert!(find_duplicates(&txs, 3, 0.9).is_empty());
    }

    #[test]
    fn description_similarity_identical() {
        assert_eq!(description_similarity("AMAZON", "AMAZON"), 1.0);
    }

    #[test]
    fn description_similarity_completely_different() {
        let score = description_similarity("AMAZON", "STARBUCKS");
        assert!(score < 0.5, "score was {score}");
    }
}
