use serde_json::json;

use crate::protocol::{ToolDefinition, ToolResult};
use crate::tools::{Access, ToolRegistry};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        ToolDefinition {
            name: "aequi_list_contacts".to_string(),
            description: "List contacts (customers, vendors, contractors) by name, with the ids used as contact_id by other tools".to_string(),
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        Access::Read,
        |db, _params| async move {
            let contacts = aequi_storage::get_all_contacts(&db).await?;
            let rows: Vec<_> = contacts
                .iter()
                .map(|c| {
                    json!({
                        "id": c.id,
                        "name": c.name,
                        "contact_type": c.contact_type,
                        "is_contractor": c.is_contractor,
                        "email": c.email
                    })
                })
                .collect();
            Ok(ToolResult::json(&rows))
        },
    );
}
