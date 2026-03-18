use serde::{Deserialize, Serialize};

use super::rules::TaxRules;

/// Metadata for a community-submitted tax rules file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxRulesMeta {
    pub country: String,
    pub jurisdiction: String,
    pub year: u16,
    pub author: String,
    pub version: u32,
    pub source_url: Option<String>,
    pub notes: Option<String>,
}

/// A community tax rules package: metadata + the rules themselves.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityTaxRules {
    pub meta: TaxRulesMeta,
    pub rules_toml: String,
}

/// Errors validating a community submission.
#[derive(Debug, thiserror::Error)]
pub enum CommunityError {
    #[error("metadata missing required field: {0}")]
    MissingField(String),
    #[error("rules parse error: {0}")]
    ParseError(String),
    #[error("year mismatch: meta says {meta_year} but rules say {rules_year}")]
    YearMismatch { meta_year: u16, rules_year: u16 },
    #[error("invalid jurisdiction: {0}")]
    InvalidJurisdiction(String),
}

/// Supported jurisdictions.
const VALID_JURISDICTIONS: &[&str] = &["us-federal", "ca", "ny", "tx", "fl", "wa"];

/// Validate a community tax rules submission.
pub fn validate_submission(submission: &CommunityTaxRules) -> Result<TaxRules, CommunityError> {
    let meta = &submission.meta;

    if meta.country.is_empty() {
        return Err(CommunityError::MissingField("country".into()));
    }
    if meta.jurisdiction.is_empty() {
        return Err(CommunityError::MissingField("jurisdiction".into()));
    }
    if meta.author.is_empty() {
        return Err(CommunityError::MissingField("author".into()));
    }

    let jurisdiction_key = format!(
        "{}-{}",
        meta.country.to_lowercase(),
        meta.jurisdiction.to_lowercase()
    );
    if meta.jurisdiction != "federal"
        && !VALID_JURISDICTIONS.contains(&jurisdiction_key.as_str())
        && !VALID_JURISDICTIONS.contains(&meta.jurisdiction.to_lowercase().as_str())
    {
        return Err(CommunityError::InvalidJurisdiction(
            meta.jurisdiction.clone(),
        ));
    }

    let rules = TaxRules::from_toml(&submission.rules_toml)
        .map_err(|e| CommunityError::ParseError(e.to_string()))?;

    if rules.year.value != meta.year {
        return Err(CommunityError::YearMismatch {
            meta_year: meta.year,
            rules_year: rules.year.value,
        });
    }

    Ok(rules)
}

/// Generate the canonical file path for a tax rules file.
pub fn rules_path(country: &str, jurisdiction: &str, year: u16) -> String {
    format!(
        "rules/tax/{}/{}/{}.toml",
        country.to_lowercase(),
        jurisdiction.to_lowercase(),
        year
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_meta() -> TaxRulesMeta {
        TaxRulesMeta {
            country: "us".to_string(),
            jurisdiction: "federal".to_string(),
            year: 2026,
            author: "test-contributor".to_string(),
            version: 1,
            source_url: Some("https://irs.gov".to_string()),
            notes: None,
        }
    }

    #[test]
    fn validate_valid_submission() {
        let submission = CommunityTaxRules {
            meta: sample_meta(),
            rules_toml: include_str!("../../test_data/tax_rules_2026.toml").to_string(),
        };
        let result = validate_submission(&submission);
        assert!(result.is_ok());
    }

    #[test]
    fn year_mismatch_rejected() {
        let mut meta = sample_meta();
        meta.year = 2025;
        let submission = CommunityTaxRules {
            meta,
            rules_toml: include_str!("../../test_data/tax_rules_2026.toml").to_string(),
        };
        let result = validate_submission(&submission);
        assert!(matches!(result, Err(CommunityError::YearMismatch { .. })));
    }

    #[test]
    fn empty_author_rejected() {
        let mut meta = sample_meta();
        meta.author = String::new();
        let submission = CommunityTaxRules {
            meta,
            rules_toml: include_str!("../../test_data/tax_rules_2026.toml").to_string(),
        };
        assert!(matches!(
            validate_submission(&submission),
            Err(CommunityError::MissingField(_))
        ));
    }

    #[test]
    fn rules_path_formatting() {
        assert_eq!(
            rules_path("US", "Federal", 2026),
            "rules/tax/us/federal/2026.toml"
        );
    }

    #[test]
    fn meta_serde_roundtrip() {
        let meta = sample_meta();
        let json = serde_json::to_string(&meta).unwrap();
        let restored: TaxRulesMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.year, 2026);
        assert_eq!(restored.author, "test-contributor");
    }
}
