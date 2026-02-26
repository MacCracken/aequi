pub mod csv;
pub mod match_engine;
pub mod ofx;
pub mod rules;
pub(crate) mod util;

pub use csv::{CsvImportProfile, CsvTransaction};
pub use match_engine::{AutoMatchEngine, MatchResult, MatchType, MatchableTransaction};
pub use ofx::{OfxStatement, OfxTransaction};
pub use rules::{CategoryRule, CategoryRuleEngine, CategorizableTransaction, MatchType as RuleMatchType};

pub mod import {
    use crate::*;
    
    pub fn import_ofx(data: &[u8]) -> Result<OfxStatement, crate::ofx::OfxError> {
        crate::ofx::parse(data)
    }

    pub fn import_csv_with_profile<R: std::io::Read>(
        data: R,
        profile: &CsvImportProfile,
    ) -> Result<Vec<CsvTransaction>, crate::csv::CsvError> {
        crate::csv::import_csv(data, profile)
    }

    pub fn create_categorization_engine(rules: Vec<CategoryRule>) -> CategoryRuleEngine {
        CategoryRuleEngine::new(rules)
    }

    pub fn create_auto_matcher(
        date_window_days: i32,
        fuzzy_threshold: f32,
        amount_tolerance_cents: i64,
    ) -> AutoMatchEngine {
        AutoMatchEngine::new(date_window_days, fuzzy_threshold, amount_tolerance_cents)
    }
}
