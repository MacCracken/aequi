-- V003: Store each quarter's installment separately from the projected annual
-- tax, and correct the Schedule C lines of the default chart of accounts.

ALTER TABLE tax_periods ADD COLUMN quarterly_payment_cents INTEGER NOT NULL DEFAULT 0;

-- Only re-tag accounts that still carry the original (incorrect) default tag,
-- so lines a user has changed deliberately are left alone.
UPDATE accounts SET schedule_c_line = 'line_1' WHERE code = '4010' AND schedule_c_line = 'line_2';
UPDATE accounts SET schedule_c_line = 'line_27' WHERE code = '5010' AND schedule_c_line = 'line_17';
UPDATE accounts SET schedule_c_line = 'line_13' WHERE code = '5040' AND schedule_c_line = 'line_15';
UPDATE accounts SET schedule_c_line = 'line_15' WHERE code = '5060' AND schedule_c_line = 'line_14';
UPDATE accounts SET schedule_c_line = 'line_9' WHERE code = '5090' AND schedule_c_line = 'line_24a';
UPDATE accounts SET schedule_c_line = 'line_25' WHERE code = '5130' AND schedule_c_line = 'line_18';
UPDATE accounts SET schedule_c_line = 'line_9' WHERE code = '5140' AND schedule_c_line = 'line_24a';

-- Lookups used to deduplicate bank-feed rows and webhook events.
CREATE INDEX IF NOT EXISTS idx_imported_tx_source ON imported_transactions(source_type, source_id);
CREATE INDEX IF NOT EXISTS idx_transactions_memo ON transactions(memo);
