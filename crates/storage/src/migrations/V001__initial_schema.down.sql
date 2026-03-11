-- V001 rollback: Drop all initial tables in reverse dependency order

DROP TABLE IF EXISTS tax_periods;
DROP TABLE IF EXISTS audit_log;
DROP TABLE IF EXISTS payments;
DROP TABLE IF EXISTS invoice_tax_lines;
DROP TABLE IF EXISTS invoice_lines;
DROP TABLE IF EXISTS invoices;
DROP TABLE IF EXISTS contacts;
DROP TABLE IF EXISTS receipt_line_items;
DROP TABLE IF EXISTS receipts;
DROP TABLE IF EXISTS reconciliation_items;
DROP TABLE IF EXISTS reconciliation_sessions;
DROP TABLE IF EXISTS categorization_rules;
DROP TABLE IF EXISTS imported_transactions;
DROP TABLE IF EXISTS import_profiles;
DROP TABLE IF EXISTS fiscal_periods;
DROP TABLE IF EXISTS settings;
DROP TABLE IF EXISTS transaction_lines;
DROP TABLE IF EXISTS transactions;
DROP TABLE IF EXISTS accounts;
