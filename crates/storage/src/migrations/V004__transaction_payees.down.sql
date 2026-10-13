-- Rollback V004

DROP INDEX IF EXISTS idx_transactions_contact;
ALTER TABLE transactions DROP COLUMN contact_id;
