mod accounts;
mod import;
mod invoices;
mod receipts;
mod reconciliation;
mod rules;
mod tax;
mod transactions;

use serde_json::Value;

use crate::protocol::{ToolDefinition, ToolResult};

pub struct ToolRegistry {
    tools: Vec<ToolEntry>,
}

type ToolHandler = Box<
    dyn Fn(
            &aequi_storage::DbPool,
            Value,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolResult> + Send>>
        + Send
        + Sync,
>;

struct ToolEntry {
    pub definition: ToolDefinition,
    pub is_write: bool,
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

    pub fn register<F, Fut>(&mut self, def: ToolDefinition, is_write: bool, handler: F)
    where
        F: Fn(aequi_storage::DbPool, Value) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ToolResult> + Send + 'static,
    {
        self.tools.push(ToolEntry {
            definition: def,
            is_write,
            handler: Box::new(move |db, params| {
                let db = db.clone();
                Box::pin(handler(db, params))
            }),
        });
    }

    pub fn list_definitions(&self) -> Vec<&ToolDefinition> {
        self.tools.iter().map(|t| &t.definition).collect()
    }

    pub async fn call(
        &self,
        name: &str,
        params: Value,
        db: &aequi_storage::DbPool,
        permissions: &crate::permissions::Permissions,
    ) -> ToolResult {
        let entry = match self.tools.iter().find(|t| t.definition.name == name) {
            Some(e) => e,
            None => return ToolResult::error(format!("Unknown tool: {name}")),
        };

        if !permissions.is_allowed(name, entry.is_write) {
            return ToolResult::error(format!("Tool {name} is not allowed"));
        }

        (entry.handler)(db, params).await
    }
}
