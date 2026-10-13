use chrono::NaiveDate;

use crate::util::similarity;

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

    /// Match imported transactions against existing ledger transactions.
    ///
    /// Each existing transaction is matched at most once, so two identical
    /// same-day charges cannot both be absorbed by a single ledger entry.
    /// Pairs are assigned greedily, best confidence first; ties go to the
    /// closer amount, then the closer date, then the earliest imported and
    /// earliest existing transaction, so the outcome is deterministic.
    /// Results are returned in the order of `imported`.
    pub fn find_matches(
        &self,
        imported: &[MatchableTransaction],
        existing: &[MatchableTransaction],
    ) -> Vec<MatchResult> {
        let tolerance = self.amount_tolerance_cents.max(0);

        // Sort existing by amount so each import only scores candidates whose
        // amount is within tolerance.
        let mut by_amount: Vec<usize> = (0..existing.len()).collect();
        by_amount.sort_by_key(|&i| (existing[i].amount_cents, i));

        let mut candidates = Vec::new();
        for (imp_idx, imp) in imported.iter().enumerate() {
            let lo = imp.amount_cents.saturating_sub(tolerance);
            let hi = imp.amount_cents.saturating_add(tolerance);
            let start = by_amount.partition_point(|&i| existing[i].amount_cents < lo);
            for &exp_idx in by_amount[start..]
                .iter()
                .take_while(|&&i| existing[i].amount_cents <= hi)
            {
                if let Some(scored) = self.score_pair(imp, &existing[exp_idx]) {
                    candidates.push((imp_idx, exp_idx, scored));
                }
            }
        }

        candidates.sort_by(|(ai, ae, a), (bi, be, b)| {
            b.confidence
                .total_cmp(&a.confidence)
                .then(a.difference_cents.cmp(&b.difference_cents))
                .then(a.date_diff_days.cmp(&b.date_diff_days))
                .then(ai.cmp(bi))
                .then(ae.cmp(be))
        });

        let mut results: Vec<MatchResult> = imported
            .iter()
            .map(|imp| MatchResult {
                imported_tx_id: imp.id,
                matched_tx_id: None,
                match_type: MatchType::None,
                confidence: 0.0,
                difference_cents: 0,
            })
            .collect();
        let mut imported_done = vec![false; imported.len()];
        let mut existing_used = vec![false; existing.len()];

        for (imp_idx, exp_idx, scored) in candidates {
            if imported_done[imp_idx] || existing_used[exp_idx] {
                continue;
            }
            imported_done[imp_idx] = true;
            existing_used[exp_idx] = true;
            let result = &mut results[imp_idx];
            result.matched_tx_id = Some(existing[exp_idx].id);
            result.match_type = scored.match_type;
            result.confidence = scored.confidence;
            result.difference_cents = scored.difference_cents;
        }

        results
    }

    /// Scores a pair if it clears the amount tolerance, date window and fuzzy
    /// threshold, else `None`.
    fn score_pair(&self, imp: &MatchableTransaction, exp: &MatchableTransaction) -> Option<Scored> {
        let diff = imp.amount_cents.abs_diff(exp.amount_cents);
        if diff > self.amount_tolerance_cents.max(0).unsigned_abs() {
            return None;
        }

        let window = u64::from(self.date_window_days.max(0).unsigned_abs());
        let date_diff = (imp.date - exp.date).num_days().unsigned_abs();
        if date_diff > window {
            return None;
        }

        let desc_score = description_similarity(&imp.description, &exp.description);
        let difference_cents = i64::try_from(diff).unwrap_or(i64::MAX);

        // Same day and same amount always matches (bank and ledger wording
        // often differ), but the description still ranks competing candidates.
        if date_diff == 0 && diff == 0 {
            return Some(Scored {
                match_type: MatchType::Exact,
                confidence: 1.0 - 0.1 * (1.0 - desc_score),
                difference_cents,
                date_diff_days: date_diff,
            });
        }

        let date_score = 1.0 - (date_diff as f32 / (window + 1) as f32);
        let confidence = (date_score + desc_score) / 2.0;

        if confidence >= self.fuzzy_threshold {
            let match_type = if date_diff == 0 {
                MatchType::DateAndAmount
            } else {
                MatchType::Fuzzy { score: confidence }
            };
            Some(Scored {
                match_type,
                confidence,
                difference_cents,
                date_diff_days: date_diff,
            })
        } else {
            None
        }
    }
}

struct Scored {
    match_type: MatchType,
    confidence: f32,
    difference_cents: i64,
    date_diff_days: u64,
}

/// Normalises a description to lowercase alphanumeric words and computes
/// Levenshtein similarity in the range [0.0, 1.0].
fn description_similarity(s1: &str, s2: &str) -> f32 {
    let a = normalize(s1);
    let b = normalize(s2);

    if a == b {
        return 1.0;
    }

    similarity(&a, &b)
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
    fn one_existing_cannot_absorb_two_identical_imports() {
        // Two real same-day $5 charges; the ledger has only one of them.
        let engine = AutoMatchEngine::default();
        let imported = vec![
            tx(1, (2024, 1, 15), "STARBUCKS", -500),
            tx(2, (2024, 1, 15), "STARBUCKS", -500),
        ];
        let existing = vec![tx(100, (2024, 1, 15), "STARBUCKS", -500)];
        let results = engine.find_matches(&imported, &existing);
        assert_eq!(results[0].matched_tx_id, Some(100));
        assert_eq!(results[1].matched_tx_id, None);
        assert_eq!(results[1].match_type, MatchType::None);
    }

    #[test]
    fn assignment_is_global_not_first_come() {
        // Import 1 is processed first but its best partner is 101; it must not
        // steal 100, which is import 2's only exact-description partner.
        let engine = AutoMatchEngine::default();
        let imported = vec![
            tx(1, (2024, 1, 15), "UBER TRIP", 500),
            tx(2, (2024, 1, 15), "STARBUCKS", 500),
        ];
        let existing = vec![
            tx(100, (2024, 1, 15), "STARBUCKS", 500),
            tx(101, (2024, 1, 15), "UBER TRIP", 500),
        ];
        let results = engine.find_matches(&imported, &existing);
        assert_eq!(results[0].matched_tx_id, Some(101));
        assert_eq!(results[1].matched_tx_id, Some(100));
    }

    #[test]
    fn second_import_falls_back_to_next_candidate() {
        let engine = AutoMatchEngine::default();
        let imported = vec![
            tx(1, (2024, 1, 15), "STARBUCKS", 500),
            tx(2, (2024, 1, 15), "STARBUCKS", 500),
        ];
        let existing = vec![
            tx(100, (2024, 1, 15), "STARBUCKS", 500),
            tx(101, (2024, 1, 16), "STARBUCKS", 500),
        ];
        let results = engine.find_matches(&imported, &existing);
        assert_eq!(results[0].matched_tx_id, Some(100));
        assert_eq!(results[0].match_type, MatchType::Exact);
        assert_eq!(results[1].matched_tx_id, Some(101));
        assert!(matches!(results[1].match_type, MatchType::Fuzzy { .. }));
    }

    #[test]
    fn exact_tier_uses_description_as_tie_breaker() {
        let engine = AutoMatchEngine::default();
        let imported = vec![tx(1, (2024, 1, 15), "AMAZON", 1000)];
        // Wrong description listed last so a "last max wins" bug would pick it.
        let existing = vec![
            tx(100, (2024, 1, 15), "AMAZON", 1000),
            tx(101, (2024, 1, 15), "TOTALLY DIFFERENT", 1000),
        ];
        let results = engine.find_matches(&imported, &existing);
        assert_eq!(results[0].matched_tx_id, Some(100));
        assert_eq!(results[0].confidence, 1.0);

        // An exact date/amount hit with unrelated wording is still a match,
        // but with lower confidence.
        let only_bad = vec![tx(101, (2024, 1, 15), "TOTALLY DIFFERENT", 1000)];
        let results = engine.find_matches(&imported, &only_bad);
        assert_eq!(results[0].matched_tx_id, Some(101));
        assert_eq!(results[0].match_type, MatchType::Exact);
        assert!(results[0].confidence < 1.0 && results[0].confidence >= 0.9);
    }

    #[test]
    fn equal_candidates_resolve_to_first() {
        let engine = AutoMatchEngine::default();
        let imported = vec![tx(1, (2024, 1, 15), "AMAZON", 1000)];
        let existing = vec![
            tx(100, (2024, 1, 15), "AMAZON", 1000),
            tx(101, (2024, 1, 15), "AMAZON", 1000),
            tx(102, (2024, 1, 15), "AMAZON", 1000),
        ];
        let results = engine.find_matches(&imported, &existing);
        assert_eq!(results[0].matched_tx_id, Some(100));
    }

    #[test]
    fn results_follow_imported_order() {
        let engine = AutoMatchEngine::default();
        let imported = vec![
            tx(3, (2024, 1, 20), "NOTHING", 7),
            tx(1, (2024, 1, 15), "AMAZON", 1000),
        ];
        let existing = vec![tx(100, (2024, 1, 15), "AMAZON", 1000)];
        let results = engine.find_matches(&imported, &existing);
        assert_eq!(results[0].imported_tx_id, 3);
        assert_eq!(results[0].matched_tx_id, None);
        assert_eq!(results[1].imported_tx_id, 1);
        assert_eq!(results[1].matched_tx_id, Some(100));
    }

    #[test]
    fn amount_tolerance_window_is_respected() {
        let engine = AutoMatchEngine::new(3, 0.7, 5);
        let imported = vec![tx(1, (2024, 1, 15), "AMAZON", 1000)];
        let existing = vec![
            tx(100, (2024, 1, 15), "AMAZON", 1006),
            tx(101, (2024, 1, 15), "AMAZON", 995),
        ];
        let results = engine.find_matches(&imported, &existing);
        assert_eq!(results[0].matched_tx_id, Some(101));
        assert_eq!(results[0].difference_cents, 5);
        assert_eq!(results[0].match_type, MatchType::DateAndAmount);
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
