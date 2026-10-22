-- MailSlate schema v2 (ARCHITECTURE.md §7.2, §7.3).
-- Forward-only: never edit a shipped migration; add 000N_*.sql instead.
-- v2 narrows imap_security/smtp_security CHECK to tls/starttls only.
-- Plaintext transport is rejected at the connect() boundary (CWE-319).
-- Existing rows with 'plaintext' will fail this migration; they must be
-- updated to 'tls' or 'starttls' before upgrading.

CREATE TABLE accounts_v2 (
    id                    INTEGER PRIMARY KEY,
    display_name          TEXT    NOT NULL,
    email                 TEXT    NOT NULL,
    provider              TEXT    NOT NULL CHECK (provider IN ('gmail', 'outlook', 'imap')),
    color                 TEXT,
    imap_host             TEXT,
    imap_port             INTEGER,
    imap_security         TEXT    CHECK (imap_security IN ('tls', 'starttls')),
    smtp_host             TEXT,
    smtp_port             INTEGER,
    smtp_security         TEXT    CHECK (smtp_security IN ('tls', 'starttls')),
    auth_type             TEXT    NOT NULL CHECK (auth_type IN ('oauth2', 'password')),
    sync_window_days      INTEGER NOT NULL DEFAULT 365,
    notifications_enabled INTEGER NOT NULL DEFAULT 1,
    archive_mailbox_id    INTEGER,
    created_at            INTEGER NOT NULL
);
INSERT INTO accounts_v2 SELECT * FROM accounts;
DROP TABLE accounts;
ALTER TABLE accounts_v2 RENAME TO accounts;
