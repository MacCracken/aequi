use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountId(pub i64);

impl fmt::Display for AccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountType {
    Asset,
    Liability,
    Equity,
    Income,
    Expense,
}

impl fmt::Display for AccountType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AccountType::Asset => write!(f, "Asset"),
            AccountType::Liability => write!(f, "Liability"),
            AccountType::Equity => write!(f, "Equity"),
            AccountType::Income => write!(f, "Income"),
            AccountType::Expense => write!(f, "Expense"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: Option<AccountId>,
    pub code: String,
    pub name: String,
    pub account_type: AccountType,
    pub is_archetype: bool,
    pub is_archived: bool,
    pub schedule_c_line: Option<String>,
}

impl Account {
    pub fn new(code: &str, name: &str, account_type: AccountType) -> Self {
        Account {
            id: None,
            code: code.to_string(),
            name: name.to_string(),
            account_type,
            is_archetype: false,
            is_archived: false,
            schedule_c_line: None,
        }
    }
}

#[derive(Debug, Clone, Error)]
pub enum LedgerError {
    #[error("Unbalanced transaction: debits={0}, credits={1}")]
    Unbalanced(super::money::Money, super::money::Money),
    #[error("Transaction must have at least two lines")]
    EmptyTransaction,
    #[error("Account not found: {0}")]
    AccountNotFound(AccountId),
    #[error("Date is in a closed period")]
    ClosedPeriod,
    #[error("Account {0} is archived")]
    ArchivedAccount(AccountId),
}

pub const DEFAULT_ACCOUNTS: &[(&str, &str, AccountType, &str)] = &[
    ("1000", "Checking", AccountType::Asset, ""),
    ("1010", "Savings", AccountType::Asset, ""),
    ("1020", "Accounts Receivable", AccountType::Asset, ""),
    ("1030", "Undeposited Funds", AccountType::Asset, ""),
    ("2000", "Credit Card", AccountType::Liability, ""),
    ("2010", "Taxes Payable", AccountType::Liability, ""),
    ("3000", "Owner's Equity", AccountType::Equity, ""),
    ("3100", "Owner's Draw", AccountType::Equity, ""),
    ("4000", "Services Revenue", AccountType::Income, "line_1"),
    ("4010", "Product Sales", AccountType::Income, "line_2"),
    ("4020", "Other Income", AccountType::Income, "line_6"),
    (
        "5000",
        "Advertising & Marketing",
        AccountType::Expense,
        "line_8",
    ),
    ("5010", "Bank Fees", AccountType::Expense, "line_17"),
    (
        "5020",
        "Business Meals (50% deductible)",
        AccountType::Expense,
        "line_24b",
    ),
    (
        "5030",
        "Education & Training",
        AccountType::Expense,
        "line_27",
    ),
    ("5040", "Equipment", AccountType::Expense, "line_15"),
    ("5050", "Home Office", AccountType::Expense, "line_30"),
    ("5060", "Insurance", AccountType::Expense, "line_14"),
    ("5070", "Internet & Phone", AccountType::Expense, "line_18"),
    (
        "5080",
        "Legal & Professional",
        AccountType::Expense,
        "line_17",
    ),
    ("5090", "Mileage", AccountType::Expense, "line_24a"),
    ("5100", "Office Supplies", AccountType::Expense, "line_18"),
    (
        "5110",
        "Software & Subscriptions",
        AccountType::Expense,
        "line_18",
    ),
    ("5120", "Travel", AccountType::Expense, "line_24a"),
    ("5130", "Utilities", AccountType::Expense, "line_18"),
    ("5140", "Vehicle Expenses", AccountType::Expense, "line_24a"),
    ("5900", "Miscellaneous", AccountType::Expense, "line_27"),
];
