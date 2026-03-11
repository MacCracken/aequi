use std::collections::HashSet;

#[derive(Debug, Clone, Default)]
pub struct Permissions {
    pub read_only: bool,
    pub disabled_tools: HashSet<String>,
}

impl Permissions {
    pub fn is_allowed(&self, tool_name: &str, is_write: bool) -> bool {
        if self.disabled_tools.contains(tool_name) {
            return false;
        }
        if self.read_only && is_write {
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_allows_everything() {
        let p = Permissions::default();
        assert!(p.is_allowed("aequi_list_accounts", false));
        assert!(p.is_allowed("aequi_create_transaction", true));
    }

    #[test]
    fn read_only_blocks_writes() {
        let p = Permissions {
            read_only: true,
            disabled_tools: HashSet::new(),
        };
        assert!(p.is_allowed("aequi_list_accounts", false));
        assert!(!p.is_allowed("aequi_create_transaction", true));
    }

    #[test]
    fn disabled_tool_blocked() {
        let mut disabled = HashSet::new();
        disabled.insert("aequi_ingest_receipt".to_string());
        let p = Permissions {
            read_only: false,
            disabled_tools: disabled,
        };
        assert!(!p.is_allowed("aequi_ingest_receipt", false));
        assert!(p.is_allowed("aequi_list_accounts", false));
    }
}
