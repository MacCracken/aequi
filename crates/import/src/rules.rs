use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::util::levenshtein_distance;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryRule {
    pub name: String,
    pub priority: i32,
    pub pattern: String,
    pub match_type: MatchType,
    pub account_code: String,
    pub amount_min_cents: Option<i64>,
    pub amount_max_cents: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum MatchType {
    #[default]
    Contains,
    Exact,
    Regex,
    Fuzzy {
        threshold: f32,
    },
}

impl std::str::FromStr for MatchType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "contains" => Ok(MatchType::Contains),
            "exact" => Ok(MatchType::Exact),
            "regex" => Ok(MatchType::Regex),
            s if s.starts_with("fuzzy:") => {
                let threshold = s[6..]
                    .parse::<f32>()
                    .map_err(|_| "Invalid fuzzy threshold".to_string())?;
                Ok(MatchType::Fuzzy { threshold })
            }
            other => Err(format!("Unknown match type: '{other}'")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CategorizableTransaction {
    pub date: NaiveDate,
    pub description: String,
    pub amount_cents: i64,
    pub memo: Option<String>,
}

/// Internal pairing of a rule with its precompiled regex (if applicable).
struct CompiledRule {
    rule: CategoryRule,
    compiled_regex: Option<regex::Regex>,
}

pub struct CategoryRuleEngine {
    rules: Vec<CompiledRule>,
}

impl CategoryRuleEngine {
    pub fn new(rules: Vec<CategoryRule>) -> Self {
        let mut compiled: Vec<CompiledRule> = rules
            .into_iter()
            .map(|rule| {
                let compiled_regex = if let MatchType::Regex = &rule.match_type {
                    regex::Regex::new(&rule.pattern).ok()
                } else {
                    None
                };
                CompiledRule { rule, compiled_regex }
            })
            .collect();
        // Highest priority first.
        compiled.sort_by(|a, b| b.rule.priority.cmp(&a.rule.priority));
        Self { rules: compiled }
    }

    pub fn from_toml(toml_content: &str) -> Result<Self, String> {
        let rules: Vec<CategoryRule> =
            toml::from_str(toml_content).map_err(|e| format!("Failed to parse TOML: {e}"))?;
        Ok(Self::new(rules))
    }

    pub fn find_matching_rule(&self, tx: &CategorizableTransaction) -> Option<&CategoryRule> {
        self.rules
            .iter()
            .find(|cr| self.rule_matches(cr, tx))
            .map(|cr| &cr.rule)
    }

    /// Returns indices + matched rules for all transactions, in order.
    pub fn apply_rules<'a>(
        &'a self,
        transactions: &[CategorizableTransaction],
    ) -> Vec<(usize, &'a CategoryRule)> {
        transactions
            .iter()
            .enumerate()
            .filter_map(|(idx, tx)| self.find_matching_rule(tx).map(|r| (idx, r)))
            .collect()
    }

    fn rule_matches(&self, cr: &CompiledRule, tx: &CategorizableTransaction) -> bool {
        let rule = &cr.rule;

        // Optional amount range filter.
        if let Some(min) = rule.amount_min_cents {
            if tx.amount_cents < min {
                return false;
            }
        }
        if let Some(max) = rule.amount_max_cents {
            if tx.amount_cents > max {
                return false;
            }
        }

        let text = tx.description.to_lowercase();
        let pattern = rule.pattern.to_lowercase();

        match &rule.match_type {
            MatchType::Contains => text.contains(&pattern),
            MatchType::Exact => text == pattern,
            MatchType::Regex => cr
                .compiled_regex
                .as_ref()
                .is_some_and(|re| re.is_match(&tx.description)),
            MatchType::Fuzzy { threshold } => fuzzy_score(&text, &pattern) >= *threshold,
        }
    }
}

fn fuzzy_score(s1: &str, s2: &str) -> f32 {
    let max_len = s1.len().max(s2.len());
    if max_len == 0 {
        return 1.0;
    }
    1.0 - (levenshtein_distance(s1, s2) as f32 / max_len as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tx(desc: &str, amount_cents: i64) -> CategorizableTransaction {
        CategorizableTransaction {
            date: chrono::NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
            description: desc.to_string(),
            amount_cents,
            memo: None,
        }
    }

    fn make_rule(pattern: &str, match_type: MatchType, account: &str, priority: i32) -> CategoryRule {
        CategoryRule {
            name: "test".to_string(),
            priority,
            pattern: pattern.to_string(),
            match_type,
            account_code: account.to_string(),
            amount_min_cents: None,
            amount_max_cents: None,
        }
    }

    #[test]
    fn contains_match_case_insensitive() {
        let engine = CategoryRuleEngine::new(vec![make_rule(
            "whole foods",
            MatchType::Contains,
            "5000",
            1,
        )]);
        let tx = make_tx("WHOLE FOODS MARKET 123", 5000);
        assert!(engine.find_matching_rule(&tx).is_some());
    }

    #[test]
    fn contains_no_match() {
        let engine = CategoryRuleEngine::new(vec![make_rule(
            "whole foods",
            MatchType::Contains,
            "5000",
            1,
        )]);
        let tx = make_tx("STARBUCKS", 500);
        assert!(engine.find_matching_rule(&tx).is_none());
    }

    #[test]
    fn exact_match() {
        let engine = CategoryRuleEngine::new(vec![make_rule(
            "starbucks",
            MatchType::Exact,
            "5020",
            1,
        )]);
        assert!(engine.find_matching_rule(&make_tx("STARBUCKS", 500)).is_some());
        assert!(engine.find_matching_rule(&make_tx("STARBUCKS RESERVE", 500)).is_none());
    }

    #[test]
    fn regex_match() {
        let engine = CategoryRuleEngine::new(vec![make_rule(
            r"^AMZN|AMAZON",
            MatchType::Regex,
            "5100",
            1,
        )]);
        assert!(engine.find_matching_rule(&make_tx("AMAZON MARKETPLACE", 1999)).is_some());
        assert!(engine.find_matching_rule(&make_tx("AMZN*PRIME", 1399)).is_some());
        assert!(engine.find_matching_rule(&make_tx("WHOLE FOODS", 1000)).is_none());
    }

    #[test]
    fn fuzzy_match_similar_strings() {
        let engine = CategoryRuleEngine::new(vec![make_rule(
            "starbucks",
            MatchType::Fuzzy { threshold: 0.8 },
            "5020",
            1,
        )]);
        // "starbuck" is 1 edit from "starbucks" → score ≈ 0.89
        assert!(engine.find_matching_rule(&make_tx("starbuck", 500)).is_some());
        // Completely unrelated — no match
        assert!(engine.find_matching_rule(&make_tx("WHOLE FOODS", 500)).is_none());
    }

    #[test]
    fn priority_ordering_highest_wins() {
        let rules = vec![
            make_rule("amazon", MatchType::Contains, "5100", 1),
            make_rule("amazon", MatchType::Contains, "5110", 10),
        ];
        let engine = CategoryRuleEngine::new(rules);
        let tx = make_tx("AMAZON MARKETPLACE", 999);
        let rule = engine.find_matching_rule(&tx).unwrap();
        assert_eq!(rule.account_code, "5110"); // higher priority wins
    }

    #[test]
    fn amount_min_filter() {
        let rule = CategoryRule {
            name: "big purchase".to_string(),
            priority: 1,
            pattern: "amazon".to_string(),
            match_type: MatchType::Contains,
            account_code: "5040".to_string(),
            amount_min_cents: Some(10_000),
            amount_max_cents: None,
        };
        let engine = CategoryRuleEngine::new(vec![rule]);
        // Below minimum — no match
        assert!(engine.find_matching_rule(&make_tx("AMAZON", 9_999)).is_none());
        // At minimum — matches
        assert!(engine.find_matching_rule(&make_tx("AMAZON", 10_000)).is_some());
    }

    #[test]
    fn amount_max_filter() {
        let rule = CategoryRule {
            name: "small purchase".to_string(),
            priority: 1,
            pattern: "amazon".to_string(),
            match_type: MatchType::Contains,
            account_code: "5100".to_string(),
            amount_min_cents: None,
            amount_max_cents: Some(500),
        };
        let engine = CategoryRuleEngine::new(vec![rule]);
        assert!(engine.find_matching_rule(&make_tx("AMAZON", 499)).is_some());
        assert!(engine.find_matching_rule(&make_tx("AMAZON", 501)).is_none());
    }

    #[test]
    fn apply_rules_returns_matched_indices() {
        let engine = CategoryRuleEngine::new(vec![make_rule(
            "github",
            MatchType::Contains,
            "5110",
            1,
        )]);
        let txs = vec![
            make_tx("GITHUB SUBSCRIPTION", 1000),
            make_tx("STARBUCKS", 500),
            make_tx("GITHUB ACTIONS", 200),
        ];
        let results = engine.apply_rules(&txs);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, 0);
        assert_eq!(results[1].0, 2);
    }

    #[test]
    fn fuzzy_score_identical_is_one() {
        assert_eq!(fuzzy_score("starbucks", "starbucks"), 1.0);
    }

    #[test]
    fn fuzzy_score_empty_strings() {
        assert_eq!(fuzzy_score("", ""), 1.0);
    }
}
