use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactId(pub i64);

impl fmt::Display for ContactId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContactType {
    Client,
    Vendor,
    Contractor,
}

impl fmt::Display for ContactType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContactType::Client => write!(f, "Client"),
            ContactType::Vendor => write!(f, "Vendor"),
            ContactType::Contractor => write!(f, "Contractor"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contact {
    pub id: Option<ContactId>,
    pub name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub address: Option<String>,
    pub contact_type: ContactType,
    pub is_contractor: bool,
    pub tax_id: Option<String>,
    pub notes: Option<String>,
}

impl Contact {
    pub fn new(name: &str, contact_type: ContactType) -> Self {
        let is_contractor = contact_type == ContactType::Contractor;
        Contact {
            id: None,
            name: name.to_string(),
            email: None,
            phone: None,
            address: None,
            contact_type,
            is_contractor,
            tax_id: None,
            notes: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contact_new_defaults() {
        let c = Contact::new("Acme Corp", ContactType::Client);
        assert_eq!(c.name, "Acme Corp");
        assert_eq!(c.contact_type, ContactType::Client);
        assert!(!c.is_contractor);
        assert!(c.id.is_none());
    }

    #[test]
    fn contractor_flag_auto_set() {
        let c = Contact::new("Jane Doe", ContactType::Contractor);
        assert!(c.is_contractor);
    }

    #[test]
    fn contact_type_display() {
        assert_eq!(ContactType::Client.to_string(), "Client");
        assert_eq!(ContactType::Vendor.to_string(), "Vendor");
        assert_eq!(ContactType::Contractor.to_string(), "Contractor");
    }

    #[test]
    fn contact_id_display() {
        assert_eq!(ContactId(7).to_string(), "7");
    }
}
