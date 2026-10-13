use std::cmp::Reverse;

use serde_json::json;

use crate::protocol::{ToolDefinition, ToolResult};
use crate::tools::args::{Args, ToolError};
use crate::tools::{Access, ToolRegistry};

/// Minimum similarity (0–1) for a `fuzzy` rule; `fuzzy:<threshold>` overrides it.
const DEFAULT_FUZZY_THRESHOLD: f64 = 0.8;
const MAX_PATTERN_LEN: usize = 500;

/// A rule's match type and pattern, validated and ready to evaluate.
/// All match types are case-insensitive.
#[derive(Debug)]
pub(crate) enum Matcher {
    Contains(String),
    Exact(String),
    Regex(regex::Regex),
    Fuzzy { pattern: String, threshold: f64 },
}

impl Matcher {
    pub(crate) fn compile(match_type: &str, pattern: &str) -> Result<Self, String> {
        if pattern.is_empty() {
            return Err("match_pattern must not be empty".to_string());
        }
        if pattern.chars().count() > MAX_PATTERN_LEN {
            return Err(format!(
                "match_pattern must be at most {MAX_PATTERN_LEN} characters"
            ));
        }
        let lowered = pattern.to_lowercase();
        match match_type.trim().to_lowercase().as_str() {
            "contains" => Ok(Matcher::Contains(lowered)),
            "exact" => Ok(Matcher::Exact(lowered)),
            "regex" => regex::RegexBuilder::new(pattern)
                .case_insensitive(true)
                .size_limit(1 << 20)
                .build()
                .map(Matcher::Regex)
                .map_err(|e| format!("Invalid regex: {e}")),
            "fuzzy" => Ok(Matcher::Fuzzy {
                pattern: lowered,
                threshold: DEFAULT_FUZZY_THRESHOLD,
            }),
            other => match other.strip_prefix("fuzzy:") {
                Some(t) => match t.parse::<f64>() {
                    Ok(threshold) if threshold > 0.0 && threshold <= 1.0 => Ok(Matcher::Fuzzy {
                        pattern: lowered,
                        threshold,
                    }),
                    _ => Err(format!(
                        "Invalid fuzzy threshold {t:?}; expected a number in (0, 1]"
                    )),
                },
                None => Err(format!(
                    "Unknown match_type {other:?}; expected contains, exact, regex or fuzzy"
                )),
            },
        }
    }

    pub(crate) fn matches(&self, description: &str) -> bool {
        match self {
            Matcher::Contains(p) => description.to_lowercase().contains(p.as_str()),
            Matcher::Exact(p) => description.to_lowercase() == *p,
            Matcher::Regex(re) => re.is_match(description),
            Matcher::Fuzzy { pattern, threshold } => {
                similarity(&description.to_lowercase(), pattern) >= *threshold
            }
        }
    }
}

/// Normalized Levenshtein similarity over characters: 1.0 for identical
/// strings, 0.0 for completely different ones.
fn similarity(a: &str, b: &str) -> f64 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let max_len = a.len().max(b.len());
    if max_len == 0 {
        return 1.0;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    1.0 - prev[b.len()] as f64 / max_len as f64
}

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        ToolDefinition {
            name: "aequi_get_categorization_rules".to_string(),
            description: "List all categorization rules".to_string(),
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        Access::Read,
        |db, _params| async move {
            let rules = aequi_storage::get_categorization_rules(&db).await?;
            Ok(ToolResult::json(&rules))
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_save_categorization_rule".to_string(),
            description: "Create a categorization rule. Matching is case-insensitive: contains/exact compare text, regex uses a regular expression, fuzzy requires 80% similarity to the whole description".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "priority": { "type": "integer", "description": "Higher priorities are tried first" },
                    "match_pattern": { "type": "string" },
                    "match_type": { "type": "string", "enum": ["contains", "exact", "regex", "fuzzy"] },
                    "account_id": { "type": "integer" }
                },
                "required": ["name", "priority", "match_pattern", "match_type", "account_id"]
            }),
        },
        Access::Write,
        |db, params| async move {
            let args = Args::new(&params);
            let name = args.str("name")?;
            let priority = args.i64("priority")?;
            let priority = i32::try_from(priority).map_err(|_| {
                ToolError(format!("Argument priority is out of range: {priority}"))
            })?;
            let match_pattern = args.str("match_pattern")?;
            let match_type = args.str("match_type")?.trim().to_lowercase();
            Matcher::compile(&match_type, match_pattern).map_err(ToolError)?;
            let account_id = args.id("account_id")?;

            let account = sqlx::query_as::<_, (bool,)>("SELECT is_archived FROM accounts WHERE id = ?")
                .bind(account_id)
                .fetch_optional(&db)
                .await?;
            match account {
                None => return Err(ToolError(format!("Account {account_id} not found"))),
                Some((true,)) => return Err(ToolError(format!("Account {account_id} is archived"))),
                Some((false,)) => {}
            }

            let rule = aequi_storage::CategorizationRule {
                id: 0,
                name: name.to_string(),
                priority,
                match_pattern: match_pattern.to_string(),
                match_type,
                account_id,
                created_at: String::new(),
            };
            let id = aequi_storage::save_categorization_rule(&db, &rule).await?;
            Ok(ToolResult::text(json!({ "rule_id": id }).to_string()))
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_apply_rules".to_string(),
            description: "Apply categorization rules to uncategorized imported transactions in a batch"
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "batch_id": { "type": "string", "description": "Import batch ID to apply rules to" }
                },
                "required": ["batch_id"]
            }),
        },
        Access::Write,
        |db, params| async move {
            let batch_id = Args::new(&params).str("batch_id")?;

            let mut rules = aequi_storage::get_categorization_rules(&db).await?;
            // Highest priority first; ties go to the older rule.
            rules.sort_by_key(|r| (Reverse(r.priority), r.id));

            let mut compiled = Vec::new();
            let mut skipped = Vec::new();
            for rule in &rules {
                match Matcher::compile(&rule.match_type, &rule.match_pattern) {
                    Ok(m) => compiled.push((rule.id, m)),
                    Err(e) => skipped.push(json!({ "rule_id": rule.id, "error": e })),
                }
            }

            let pending = aequi_storage::get_pending_imported_transactions(&db, batch_id).await?;
            let matches: Vec<(i64, i64)> = pending
                .iter()
                .filter_map(|tx| {
                    compiled
                        .iter()
                        .find(|(_, m)| m.matches(&tx.description))
                        .map(|(rule_id, _)| (tx.id, *rule_id))
                })
                .collect();

            // All-or-nothing, and only rows that are still pending.
            let mut db_tx = db.begin().await?;
            let mut matched = 0u64;
            for (tx_id, rule_id) in &matches {
                matched += sqlx::query(
                    "UPDATE imported_transactions SET category_rule_id = ?, status = 'categorized' \
                     WHERE id = ? AND status = 'pending'",
                )
                .bind(rule_id)
                .bind(tx_id)
                .execute(&mut *db_tx)
                .await?
                .rows_affected();
            }
            db_tx.commit().await?;

            let total = pending.len() as u64;
            Ok(ToolResult::text(
                json!({
                    "total_pending": total,
                    "matched": matched,
                    "unmatched": total - matched,
                    "skipped_rules": skipped
                })
                .to_string(),
            ))
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regex_rules_use_regex_semantics() {
        let m = Matcher::compile("regex", r"^SAAS.*MONTHLY$").unwrap();
        assert!(m.matches("SaaS Tool Monthly"));
        assert!(!m.matches("SAAS.*MONTHLY extra"));
        assert!(!m.matches("Annual SAAS"));
    }

    #[test]
    fn invalid_regex_rejected() {
        assert!(Matcher::compile("regex", "(unclosed").is_err());
    }

    #[test]
    fn fuzzy_rules_use_similarity() {
        let m = Matcher::compile("fuzzy", "amazon web services").unwrap();
        assert!(m.matches("AMAZON WEB SERVICE"));
        assert!(!m.matches("amazon"));
        // A literal substring is not a fuzzy match on its own.
        assert!(!m.matches("payment to amazon web services inc via card 1234"));
        let strict = Matcher::compile("fuzzy:1.0", "amazon").unwrap();
        assert!(strict.matches("Amazon"));
        assert!(!strict.matches("Amazn"));
    }

    #[test]
    fn unknown_match_type_and_empty_pattern_rejected() {
        assert!(Matcher::compile("startswith", "x").is_err());
        assert!(Matcher::compile("contains", "").is_err());
        assert!(Matcher::compile("fuzzy:2", "x").is_err());
        assert!(Matcher::compile("fuzzy:abc", "x").is_err());
    }

    #[test]
    fn contains_and_exact_are_case_insensitive() {
        assert!(
            Matcher::compile("contains", "github")
                .unwrap()
                .matches("GITHUB INC")
        );
        assert!(
            Matcher::compile("exact", "github")
                .unwrap()
                .matches("GitHub")
        );
        assert!(
            !Matcher::compile("exact", "github")
                .unwrap()
                .matches("GitHub Inc")
        );
    }

    #[test]
    fn similarity_bounds() {
        assert_eq!(similarity("", ""), 1.0);
        assert_eq!(similarity("abc", "abc"), 1.0);
        assert_eq!(similarity("abc", "xyz"), 0.0);
        assert!((similarity("café", "cafe") - 0.75).abs() < 1e-9);
    }
}
