mod accounts;
pub mod args;
mod import;
mod invoices;
mod receipts;
mod reconciliation;
mod rules;
mod tax;
mod transactions;

use serde_json::Value;

use crate::permissions::{McpSettings, Permissions};
use crate::protocol::{ToolDefinition, ToolResult};
use args::ToolOutcome;
pub use receipts::MAX_RECEIPT_BYTES;

/// Whether a tool only reads the ledger or changes it. Every tool that
/// inserts, updates or deletes anything must be registered as `Write` so the
/// read-only setting blocks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

pub struct ToolRegistry {
    tools: Vec<ToolEntry>,
}

type ToolHandler = Box<
    dyn Fn(
            &aequi_storage::DbPool,
            Value,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolOutcome> + Send>>
        + Send
        + Sync,
>;

struct ToolEntry {
    pub definition: ToolDefinition,
    pub access: Access,
    pub handler: ToolHandler,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        let mut registry = ToolRegistry { tools: Vec::new() };

        accounts::register(&mut registry);
        transactions::register(&mut registry);
        receipts::register(&mut registry);
        tax::register(&mut registry);
        invoices::register(&mut registry);
        rules::register(&mut registry);
        import::register(&mut registry);
        reconciliation::register(&mut registry);

        registry
    }

    pub fn register<F, Fut>(&mut self, def: ToolDefinition, access: Access, handler: F)
    where
        F: Fn(aequi_storage::DbPool, Value) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ToolOutcome> + Send + 'static,
    {
        self.tools.push(ToolEntry {
            definition: def,
            access,
            handler: Box::new(move |db, params| {
                let db = db.clone();
                Box::pin(handler(db, params))
            }),
        });
    }

    pub fn list_definitions(&self) -> Vec<&ToolDefinition> {
        self.tools.iter().map(|t| &t.definition).collect()
    }

    /// Names of the tools registered as `Access::Write`.
    pub fn write_tools(&self) -> Vec<&str> {
        self.tools
            .iter()
            .filter(|t| t.access == Access::Write)
            .map(|t| t.definition.name.as_str())
            .collect()
    }

    pub async fn call(
        &self,
        name: &str,
        params: Value,
        db: &aequi_storage::DbPool,
        permissions: &Permissions,
    ) -> ToolResult {
        let entry = match self.tools.iter().find(|t| t.definition.name == name) {
            Some(e) => e,
            None => return ToolResult::error(format!("Unknown tool: {name}")),
        };

        // Fail closed: if the settings can't be read, nothing runs.
        let settings = match McpSettings::load(db).await {
            Ok(s) => s,
            Err(e) => return ToolResult::error(format!("Cannot read MCP settings: {e}")),
        };
        if let Err(message) = permissions.check(
            settings,
            &entry.definition.name,
            entry.access == Access::Write,
        ) {
            return ToolResult::error(message);
        }

        let params = match params {
            Value::Null => Value::Object(Default::default()),
            Value::Object(_) => params,
            _ => return ToolResult::error("Tool arguments must be a JSON object".to_string()),
        };
        if let Err(message) = check_known_arguments(&entry.definition, &params) {
            return ToolResult::error(message);
        }

        match (entry.handler)(db, params).await {
            Ok(result) => result,
            Err(e) => ToolResult::error(e.0),
        }
    }
}

/// Reject arguments the tool doesn't declare, so a misspelled optional
/// argument (e.g. a date filter) fails loudly instead of being ignored.
fn check_known_arguments(def: &ToolDefinition, params: &Value) -> Result<(), String> {
    let declared = def
        .input_schema
        .get("properties")
        .and_then(Value::as_object);
    let unknown: Vec<&str> = params
        .as_object()
        .into_iter()
        .flat_map(|m| m.keys())
        .filter(|k| !declared.is_some_and(|d| d.contains_key(k.as_str())))
        .map(String::as_str)
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    let expected: Vec<&str> = declared
        .into_iter()
        .flat_map(|d| d.keys())
        .map(String::as_str)
        .collect();
    Err(format!(
        "Unknown argument(s) for {}: {}. Expected: {}",
        def.name,
        unknown.join(", "),
        if expected.is_empty() {
            "none".to_string()
        } else {
            expected.join(", ")
        }
    ))
}
