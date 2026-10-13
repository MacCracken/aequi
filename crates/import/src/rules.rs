use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};

use crate::util::similarity;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryRule {
    pub name: String,
    pub priority: i32,
    pub pattern: String,
    pub match_type: MatchType,
    pub account_code: String,
    pub amount_min_cents: Option<i64>,
    pub amount_max_cents: Option<i64>,
    /// Memo to attach to transactions categorized by this rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
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
                CompiledRule {
                    rule,
                    compiled_regex,
                }
            })
            .collect();
        // Highest priority first.
        compiled.sort_by_key(|c| std::cmp::Reverse(c.rule.priority));
        Self { rules: compiled }
    }

    /// Load rules from a TOML rule file made of `[[rules]]` tables.
    ///
    /// Each table may use either the [`CategoryRule`] field names or the
    /// documented shorthand:
    ///
    /// ```toml
    /// [[rules]]
    /// priority = 10
    /// match_description = "GITHUB"          # substring, case-insensitive
    /// match_amount_min = -100.00            # dollars; or amount_min_cents
    /// assign_account = "5110"
    /// memo = "GitHub subscription"
    ///
    /// [[rules]]
    /// priority = 20
    /// match_description_regex = "^AMZN|AMAZON"
    /// assign_account = "5100"
    /// ```
    ///
    /// Invalid regexes, thresholds or amount ranges are reported as errors
    /// instead of producing rules that silently never match.
    pub fn from_toml(toml_content: &str) -> Result<Self, String> {
        let file: RuleFile =
            toml::from_str(toml_content).map_err(|e| format!("Failed to parse TOML: {e}"))?;
        let rules = file
            .rules
            .into_iter()
            .enumerate()
            .map(|(i, entry)| {
                entry
                    .into_rule()
                    .map_err(|e| format!("rule #{}: {e}", i + 1))
            })
            .collect::<Result<Vec<_>, _>>()?;
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
        if let Some(min) = rule.amount_min_cents
            && tx.amount_cents < min
        {
            return false;
        }
        if let Some(max) = rule.amount_max_cents
            && tx.amount_cents > max
        {
            return false;
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
    similarity(s1, s2)
}

/// Top level of a rule file.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleFile {
    #[serde(default)]
    rules: Vec<RuleEntry>,
}

/// One `[[rules]]` table, accepting both naming schemes (see
/// [`CategoryRuleEngine::from_toml`]).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleEntry {
    name: Option<String>,
    #[serde(default)]
    priority: i32,
    pattern: Option<String>,
    match_description: Option<String>,
    match_description_regex: Option<String>,
    match_type: Option<MatchTypeSpec>,
    #[serde(alias = "assign_account")]
    account_code: String,
    amount_min_cents: Option<i64>,
    amount_max_cents: Option<i64>,
    /// Dollars, e.g. `-100.00`.
    match_amount_min: Option<Decimal>,
    match_amount_max: Option<Decimal>,
    memo: Option<String>,
}

/// `match_type` as the enum's own encoding (`"Contains"`,
/// `{ Fuzzy = { threshold = 0.8 } }`) or as text (`"contains"`, `"fuzzy:0.8"`).
#[derive(Deserialize)]
#[serde(untagged)]
enum MatchTypeSpec {
    Native(MatchType),
    Text(String),
}

impl RuleEntry {
    fn into_rule(self) -> Result<CategoryRule, String> {
        let match_type = match self.match_type {
            Some(MatchTypeSpec::Native(m)) => Some(m),
            Some(MatchTypeSpec::Text(t)) => Some(t.parse::<MatchType>()?),
            None => None,
        };

        let (pattern, match_type) = match (
            self.pattern,
            self.match_description,
            self.match_description_regex,
        ) {
            (Some(p), None, None) | (None, Some(p), None) => (p, match_type.unwrap_or_default()),
            (None, None, Some(p)) => match match_type {
                None | Some(MatchType::Regex) => (p, MatchType::Regex),
                Some(_) => {
                    return Err("match_description_regex conflicts with match_type".to_string());
                }
            },
            (None, None, None) => {
                return Err(
                    "needs one of pattern, match_description or match_description_regex"
                        .to_string(),
                );
            }
            _ => {
                return Err(
                    "use only one of pattern, match_description or match_description_regex"
                        .to_string(),
                );
            }
        };
        if pattern.is_empty() {
            return Err("pattern must not be empty".to_string());
        }
        match &match_type {
            MatchType::Regex => {
                regex::Regex::new(&pattern).map_err(|e| format!("invalid regex: {e}"))?;
            }
            MatchType::Fuzzy { threshold } if !(0.0..=1.0).contains(threshold) => {
                return Err(format!("fuzzy threshold {threshold} is outside 0.0-1.0"));
            }
            _ => {}
        }

        let account_code = self.account_code.trim().to_string();
        if account_code.is_empty() {
            return Err("account_code must not be empty".to_string());
        }

        let amount_min_cents =
            amount_bound(self.amount_min_cents, self.match_amount_min, "minimum")?;
        let amount_max_cents =
            amount_bound(self.amount_max_cents, self.match_amount_max, "maximum")?;
        if let (Some(min), Some(max)) = (amount_min_cents, amount_max_cents)
            && min > max
        {
            return Err(format!(
                "amount minimum {min} exceeds maximum {max} (cents)"
            ));
        }

        Ok(CategoryRule {
            name: self.name.unwrap_or_else(|| pattern.clone()),
            priority: self.priority,
            pattern,
            match_type,
            account_code,
            amount_min_cents,
            amount_max_cents,
            memo: self.memo,
        })
    }
}

fn amount_bound(
    cents: Option<i64>,
    dollars: Option<Decimal>,
    which: &str,
) -> Result<Option<i64>, String> {
    match (cents, dollars) {
        (Some(_), Some(_)) => Err(format!(
            "give the amount {which} in cents or in dollars, not both"
        )),
        (Some(c), None) => Ok(Some(c)),
        (None, Some(d)) => {
            if d.round_dp(2) != d {
                return Err(format!("amount {which} {d} has fractions of a cent"));
            }
            d.checked_mul(Decimal::ONE_HUNDRED)
                .and_then(|c| c.to_i64())
                .map(Some)
                .ok_or_else(|| format!("amount {which} {d} is out of range"))
        }
        (None, None) => Ok(None),
    }
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

    fn make_rule(
        pattern: &str,
        match_type: MatchType,
        account: &str,
        priority: i32,
    ) -> CategoryRule {
        CategoryRule {
            name: "test".to_string(),
            priority,
            pattern: pattern.to_string(),
            match_type,
            account_code: account.to_string(),
            amount_min_cents: None,
            amount_max_cents: None,
            memo: None,
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
        let engine =
            CategoryRuleEngine::new(vec![make_rule("starbucks", MatchType::Exact, "5020", 1)]);
        assert!(
            engine
                .find_matching_rule(&make_tx("STARBUCKS", 500))
                .is_some()
        );
        assert!(
            engine
                .find_matching_rule(&make_tx("STARBUCKS RESERVE", 500))
                .is_none()
        );
    }

    #[test]
    fn regex_match() {
        let engine = CategoryRuleEngine::new(vec![make_rule(
            r"^AMZN|AMAZON",
            MatchType::Regex,
            "5100",
            1,
        )]);
        assert!(
            engine
                .find_matching_rule(&make_tx("AMAZON MARKETPLACE", 1999))
                .is_some()
        );
        assert!(
            engine
                .find_matching_rule(&make_tx("AMZN*PRIME", 1399))
                .is_some()
        );
        assert!(
            engine
                .find_matching_rule(&make_tx("WHOLE FOODS", 1000))
                .is_none()
        );
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
        assert!(
            engine
                .find_matching_rule(&make_tx("starbuck", 500))
                .is_some()
        );
        // Completely unrelated — no match
        assert!(
            engine
                .find_matching_rule(&make_tx("WHOLE FOODS", 500))
                .is_none()
        );
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
            memo: None,
        };
        let engine = CategoryRuleEngine::new(vec![rule]);
        // Below minimum — no match
        assert!(
            engine
                .find_matching_rule(&make_tx("AMAZON", 9_999))
                .is_none()
        );
        // At minimum — matches
        assert!(
            engine
                .find_matching_rule(&make_tx("AMAZON", 10_000))
                .is_some()
        );
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
            memo: None,
        };
        let engine = CategoryRuleEngine::new(vec![rule]);
        assert!(engine.find_matching_rule(&make_tx("AMAZON", 499)).is_some());
        assert!(engine.find_matching_rule(&make_tx("AMAZON", 501)).is_none());
    }

    #[test]
    fn apply_rules_returns_matched_indices() {
        let engine =
            CategoryRuleEngine::new(vec![make_rule("github", MatchType::Contains, "5110", 1)]);
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

    #[test]
    fn fuzzy_score_counts_characters_not_bytes() {
        // "café" vs "cafe" is one edit over four characters.
        assert!((fuzzy_score("café", "cafe") - 0.75).abs() < 1e-6);
    }

    /// The categorization rule example from docs/development/aequi_formalation.md §5.2.
    const DOCUMENTED_EXAMPLE: &str = r#"
[[rules]]
priority = 10
match_description = "GITHUB"          # substring, case-insensitive
match_amount_min = -100.00            # optional range filter
assign_account = "5110"               # Software & Subscriptions
memo = "GitHub subscription"

[[rules]]
priority = 20
match_description_regex = "^AMZN|AMAZON"
assign_account = "5100"               # Office Supplies (default; AI may override)
"#;

    #[test]
    fn from_toml_parses_documented_example() {
        let engine = CategoryRuleEngine::from_toml(DOCUMENTED_EXAMPLE).unwrap();
        assert_eq!(engine.rules.len(), 2);

        let github = engine
            .find_matching_rule(&make_tx("GitHub Inc. monthly", -400))
            .unwrap();
        assert_eq!(github.account_code, "5110");
        assert_eq!(github.match_type, MatchType::Contains);
        assert_eq!(github.amount_min_cents, Some(-10_000));
        assert_eq!(github.memo.as_deref(), Some("GitHub subscription"));
        // Below the -$100 minimum.
        assert!(
            engine
                .find_matching_rule(&make_tx("GITHUB ENTERPRISE", -25_000))
                .is_none()
        );

        let amazon = engine
            .find_matching_rule(&make_tx("AMZN Mktp US", -1999))
            .unwrap();
        assert_eq!(amazon.account_code, "5100");
        assert_eq!(amazon.match_type, MatchType::Regex);
        assert_eq!(amazon.priority, 20);
    }

    #[test]
    fn from_toml_accepts_struct_field_names() {
        let toml = r#"
[[rules]]
name = "Coffee"
priority = 5
pattern = "starbuck"
match_type = "fuzzy:0.8"
account_code = "5020"
amount_max_cents = 2000

[[rules]]
name = "Exact"
priority = 1
pattern = "rent"
match_type = "Exact"
account_code = "5300"
"#;
        let engine = CategoryRuleEngine::from_toml(toml).unwrap();
        let rule = engine
            .find_matching_rule(&make_tx("STARBUCKS", 500))
            .unwrap();
        assert_eq!(rule.name, "Coffee");
        assert_eq!(rule.match_type, MatchType::Fuzzy { threshold: 0.8 });
        assert!(
            engine
                .find_matching_rule(&make_tx("RENT", 100_000))
                .is_some()
        );
    }

    #[test]
    fn from_toml_roundtrips_serialized_rules() {
        #[derive(Serialize)]
        struct File<'a> {
            rules: &'a [CategoryRule],
        }
        let rules = vec![
            make_rule("github", MatchType::Contains, "5110", 3),
            make_rule("starbucks", MatchType::Fuzzy { threshold: 0.9 }, "5020", 2),
        ];
        let text = toml::to_string(&File { rules: &rules }).unwrap();
        let engine = CategoryRuleEngine::from_toml(&text).unwrap();
        assert_eq!(engine.rules.len(), 2);
        assert_eq!(
            engine.rules[1].rule.match_type,
            MatchType::Fuzzy { threshold: 0.9 }
        );
    }

    #[test]
    fn from_toml_rejects_invalid_rules() {
        let cases = [
            // bare array at the root can never be a TOML document
            "rules = 5",
            // unknown key (typo) is not silently ignored
            "[[rules]]\nmatch_descripton = \"X\"\nassign_account = \"1\"",
            // no pattern at all
            "[[rules]]\nassign_account = \"1\"",
            // two patterns
            "[[rules]]\nmatch_description = \"A\"\npattern = \"B\"\nassign_account = \"1\"",
            // regex that does not compile
            "[[rules]]\nmatch_description_regex = \"(unclosed\"\nassign_account = \"1\"",
            // fuzzy threshold out of range
            "[[rules]]\npattern = \"a\"\nmatch_type = \"fuzzy:1.5\"\naccount_code = \"1\"",
            // sub-cent dollar amount
            "[[rules]]\npattern = \"a\"\nmatch_amount_min = 1.005\naccount_code = \"1\"",
            // min above max
            "[[rules]]\npattern = \"a\"\nmatch_amount_min = 10\nmatch_amount_max = 5\naccount_code = \"1\"",
            // missing account
            "[[rules]]\npattern = \"a\"",
        ];
        for case in cases {
            assert!(CategoryRuleEngine::from_toml(case).is_err(), "{case}");
        }
    }

    #[test]
    fn from_toml_empty_file_has_no_rules() {
        let engine = CategoryRuleEngine::from_toml("").unwrap();
        assert!(engine.rules.is_empty());
    }
}
