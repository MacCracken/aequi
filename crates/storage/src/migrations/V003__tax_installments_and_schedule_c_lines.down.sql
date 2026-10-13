-- Rollback V003

DROP INDEX IF EXISTS idx_transactions_memo;
DROP INDEX IF EXISTS idx_imported_tx_source;

UPDATE accounts SET schedule_c_line = 'line_24a' WHERE code = '5140' AND schedule_c_line = 'line_9';
UPDATE accounts SET schedule_c_line = 'line_18' WHERE code = '5130' AND schedule_c_line = 'line_25';
UPDATE accounts SET schedule_c_line = 'line_24a' WHERE code = '5090' AND schedule_c_line = 'line_9';
UPDATE accounts SET schedule_c_line = 'line_14' WHERE code = '5060' AND schedule_c_line = 'line_15';
UPDATE accounts SET schedule_c_line = 'line_15' WHERE code = '5040' AND schedule_c_line = 'line_13';
UPDATE accounts SET schedule_c_line = 'line_17' WHERE code = '5010' AND schedule_c_line = 'line_27';
UPDATE accounts SET schedule_c_line = 'line_2' WHERE code = '4010' AND schedule_c_line = 'line_1';

ALTER TABLE tax_periods DROP COLUMN quarterly_payment_cents;
