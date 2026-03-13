use serde::{Deserialize, Serialize};

use crate::csv::CsvImportProfile;
use crate::rules::CategoryRule;

/// A shareable import profile bundle with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedProfile {
    pub meta: ProfileMeta,
    pub csv_profile: CsvImportProfile,
    #[serde(default)]
    pub categorization_rules: Vec<CategoryRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileMeta {
    pub name: String,
    pub description: String,
    pub institution: String,
    pub author: Option<String>,
    pub version: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileSharingError {
    #[error("failed to serialize profile: {0}")]
    Serialize(String),
    #[error("failed to parse profile: {0}")]
    Parse(String),
    #[error("validation error: {0}")]
    Validation(String),
}

/// Export a profile to a shareable TOML string.
pub fn export_profile(profile: &SharedProfile) -> Result<String, ProfileSharingError> {
    toml::to_string_pretty(profile).map_err(|e| ProfileSharingError::Serialize(e.to_string()))
}

/// Import a profile from a TOML string.
pub fn import_profile(toml_str: &str) -> Result<SharedProfile, ProfileSharingError> {
    let profile: SharedProfile =
        toml::from_str(toml_str).map_err(|e| ProfileSharingError::Parse(e.to_string()))?;

    validate_profile(&profile)?;
    Ok(profile)
}

fn validate_profile(profile: &SharedProfile) -> Result<(), ProfileSharingError> {
    if profile.meta.name.is_empty() {
        return Err(ProfileSharingError::Validation(
            "profile name is required".into(),
        ));
    }
    if profile.meta.institution.is_empty() {
        return Err(ProfileSharingError::Validation(
            "institution is required".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::csv::{CsvColumnMapping, CsvImportProfile};

    fn sample_profile() -> SharedProfile {
        SharedProfile {
            meta: ProfileMeta {
                name: "Chase Checking".to_string(),
                description: "Chase Bank checking account CSV export".to_string(),
                institution: "Chase".to_string(),
                author: Some("community".to_string()),
                version: 1,
            },
            csv_profile: CsvImportProfile {
                id: None,
                name: "Chase Checking".to_string(),
                mapping: CsvColumnMapping {
                    date_column: Some(0),
                    description_column: Some(1),
                    amount_column: Some(2),
                    debit_column: None,
                    credit_column: None,
                    memo_column: Some(3),
                    date_format: "%m/%d/%Y".to_string(),
                },
                has_header: true,
                delimiter: ",".to_string(),
            },
            categorization_rules: vec![],
        }
    }

    #[test]
    fn export_import_roundtrip() {
        let profile = sample_profile();
        let toml = export_profile(&profile).unwrap();
        let restored = import_profile(&toml).unwrap();
        assert_eq!(restored.meta.name, "Chase Checking");
        assert_eq!(restored.meta.institution, "Chase");
        assert_eq!(
            restored.csv_profile.mapping.date_format,
            "%m/%d/%Y"
        );
    }

    #[test]
    fn empty_name_rejected() {
        let mut profile = sample_profile();
        profile.meta.name = String::new();
        let toml = export_profile(&profile).unwrap();
        assert!(matches!(
            import_profile(&toml),
            Err(ProfileSharingError::Validation(_))
        ));
    }

    #[test]
    fn empty_institution_rejected() {
        let mut profile = sample_profile();
        profile.meta.institution = String::new();
        let toml = export_profile(&profile).unwrap();
        assert!(matches!(
            import_profile(&toml),
            Err(ProfileSharingError::Validation(_))
        ));
    }

    #[test]
    fn toml_contains_expected_fields() {
        let profile = sample_profile();
        let toml = export_profile(&profile).unwrap();
        assert!(toml.contains("[meta]"));
        assert!(toml.contains("[csv_profile]"));
        assert!(toml.contains("Chase"));
    }
}
