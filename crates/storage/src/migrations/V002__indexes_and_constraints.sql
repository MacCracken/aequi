-- V002: Add performance indexes for common query patterns

CREATE INDEX IF NOT EXISTS idx_transaction_lines_transaction_id ON transaction_lines(transaction_id);
CREATE INDEX IF NOT EXISTS idx_transaction_lines_account_id ON transaction_lines(account_id);
CREATE INDEX IF NOT EXISTS idx_imported_tx_batch_status ON imported_transactions(import_batch_id, status);
CREATE INDEX IF NOT EXISTS idx_imported_tx_status ON imported_transactions(status);
CREATE INDEX IF NOT EXISTS idx_invoices_status ON invoices(status_type);
CREATE INDEX IF NOT EXISTS idx_invoices_contact ON invoices(contact_id);
CREATE INDEX IF NOT EXISTS idx_receipts_status ON receipts(status);
CREATE INDEX IF NOT EXISTS idx_payments_invoice ON payments(invoice_id);
CREATE INDEX IF NOT EXISTS idx_payments_date ON payments(date);
CREATE INDEX IF NOT EXISTS idx_audit_log_timestamp ON audit_log(timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_transactions_date ON transactions(date);
CREATE INDEX IF NOT EXISTS idx_contacts_type ON contacts(contact_type);
CREATE INDEX IF NOT EXISTS idx_reconciliation_items_session ON reconciliation_items(session_id);
