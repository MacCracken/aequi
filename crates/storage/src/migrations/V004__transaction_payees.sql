-- V004: Link ledger transactions to a contact (payee/payer), so payments made
-- to contractors can be totaled for 1099-NEC reporting.

ALTER TABLE transactions ADD COLUMN contact_id INTEGER REFERENCES contacts(id);
CREATE INDEX IF NOT EXISTS idx_transactions_contact ON transactions(contact_id);
