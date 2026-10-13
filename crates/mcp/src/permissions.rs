use std::collections::HashSet;

/// Settings-table key written by the desktop Settings page. Unset means
/// enabled; only the exact value "false" turns MCP access off.
pub const SETTING_ENABLED: &str = "mcp_enabled";
/// Settings-table key written by the desktop Settings page. Only the exact
/// value "true" makes MCP access read-only.
pub const SETTING_READ_ONLY: &str = "mcp_read_only";

/// Static restrictions configured when the server starts.
#[derive(Debug, Clone, Default)]
pub struct Permissions {
    /// Refuse write tools regardless of the desktop setting.
    pub read_only: bool,
    pub disabled_tools: HashSet<String>,
}

/// The MCP toggles from the desktop app's settings table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpSettings {
    pub enabled: bool,
    pub read_only: bool,
}

impl Default for McpSettings {
    fn default() -> Self {
        McpSettings {
            enabled: true,
            read_only: false,
        }
    }
}

impl McpSettings {
    pub fn from_values(enabled: Option<&str>, read_only: Option<&str>) -> Self {
        McpSettings {
            enabled: enabled != Some("false"),
            read_only: read_only == Some("true"),
        }
    }

    /// Read the current toggles. Called on every tool call so changes made in
    /// the desktop app apply without restarting the server.
    pub async fn load(db: &aequi_storage::DbPool) -> Result<Self, sqlx::Error> {
        let enabled = aequi_storage::get_setting(db, SETTING_ENABLED).await?;
        let read_only = aequi_storage::get_setting(db, SETTING_READ_ONLY).await?;
        Ok(Self::from_values(enabled.as_deref(), read_only.as_deref()))
    }
}

impl Permissions {
    /// Decide whether a registered tool may run. `tool_name` must be the
    /// tool's canonical registered name; `Err` carries the refusal message.
    pub fn check(
        &self,
        settings: McpSettings,
        tool_name: &str,
        is_write: bool,
    ) -> Result<(), String> {
        if !settings.enabled {
            return Err(
                "MCP access is disabled in Aequi settings; enable it in the desktop app to use this tool"
                    .to_string(),
            );
        }
        if self.disabled_tools.contains(tool_name) {
            return Err(format!("Tool {tool_name} is not allowed: it is disabled"));
        }
        if is_write && (self.read_only || settings.read_only) {
            return Err(format!(
                "Tool {tool_name} is not allowed: MCP access is read-only and this tool modifies the ledger"
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_allows_everything() {
        let p = Permissions::default();
        let s = McpSettings::default();
        assert!(p.check(s, "aequi_list_accounts", false).is_ok());
        assert!(p.check(s, "aequi_create_transaction", true).is_ok());
    }

    #[test]
    fn read_only_blocks_writes() {
        let p = Permissions {
            read_only: true,
            disabled_tools: HashSet::new(),
        };
        let s = McpSettings::default();
        assert!(p.check(s, "aequi_list_accounts", false).is_ok());
        assert!(p.check(s, "aequi_create_transaction", true).is_err());
    }

    #[test]
    fn read_only_setting_blocks_writes() {
        let p = Permissions::default();
        let s = McpSettings::from_values(None, Some("true"));
        assert!(p.check(s, "aequi_list_accounts", false).is_ok());
        let err = p.check(s, "aequi_create_transaction", true).unwrap_err();
        assert!(err.contains("read-only"));
    }

    #[test]
    fn disabled_setting_blocks_everything() {
        let p = Permissions::default();
        let s = McpSettings::from_values(Some("false"), None);
        assert!(p.check(s, "aequi_list_accounts", false).is_err());
        assert!(p.check(s, "aequi_create_transaction", true).is_err());
    }

    #[test]
    fn setting_defaults() {
        assert_eq!(McpSettings::from_values(None, None), McpSettings::default());
        assert!(McpSettings::from_values(Some("true"), None).enabled);
        assert!(!McpSettings::from_values(Some("false"), None).enabled);
        assert!(!McpSettings::from_values(None, Some("false")).read_only);
        assert!(McpSettings::from_values(None, Some("true")).read_only);
    }

    #[test]
    fn disabled_tool_blocked() {
        let mut disabled = HashSet::new();
        disabled.insert("aequi_ingest_receipt".to_string());
        let p = Permissions {
            read_only: false,
            disabled_tools: disabled,
        };
        let s = McpSettings::default();
        assert!(p.check(s, "aequi_ingest_receipt", false).is_err());
        assert!(p.check(s, "aequi_list_accounts", false).is_ok());
    }
}
