-- MailSlate schema v1 (ARCHITECTURE.md §7.2, §7.3).
-- Forward-only: never edit a shipped migration; add 000N_*.sql instead.

CREATE TABLE accounts (
    id                    INTEGER PRIMARY KEY,
    display_name          TEXT    NOT NULL,
    email                 TEXT    NOT NULL,
    provider              TEXT    NOT NULL CHECK (provider IN ('gmail', 'outlook', 'imap')),
    color                 TEXT,
    imap_host             TEXT,
    imap_port             INTEGER,
    imap_security         TEXT    CHECK (imap_security IN ('tls', 'starttls', 'plaintext')),
    smtp_host             TEXT,
    smtp_port             INTEGER,
    smtp_security         TEXT    CHECK (smtp_security IN ('tls', 'starttls', 'plaintext')),
    auth_type             TEXT    NOT NULL CHECK (auth_type IN ('oauth2', 'password')),
    sync_window_days      INTEGER NOT NULL DEFAULT 365,
    notifications_enabled INTEGER NOT NULL DEFAULT 1,
    archive_mailbox_id    INTEGER,
    created_at            INTEGER NOT NULL
);

CREATE TABLE mailboxes (
    id             INTEGER PRIMARY KEY,
    account_id     INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    name           TEXT    NOT NULL,
    kind           TEXT    NOT NULL CHECK (kind IN ('inbox', 'sent', 'drafts', 'trash', 'spam', 'archive', 'all', 'important', 'user')),
    special_use    TEXT,
    color          TEXT,
    sort_order     INTEGER NOT NULL DEFAULT 0,
    uidvalidity    INTEGER,
    uidnext        INTEGER,
    highest_modseq INTEGER,
    last_synced_at INTEGER,
    UNIQUE (account_id, name)
);

CREATE TABLE threads (
    id              INTEGER PRIMARY KEY,
    account_id      INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    gmail_thread_id TEXT,
    root_message_id TEXT,
    subject_norm    TEXT,
    last_date       INTEGER NOT NULL,
    message_count   INTEGER NOT NULL DEFAULT 0,
    unread_count    INTEGER NOT NULL DEFAULT 0,
    has_attachments INTEGER NOT NULL DEFAULT 0,
    last_snippet    TEXT
);

CREATE UNIQUE INDEX idx_threads_gmail
    ON threads (account_id, gmail_thread_id)
    WHERE gmail_thread_id IS NOT NULL;

CREATE INDEX idx_threads_root ON threads (account_id, root_message_id, subject_norm);
CREATE INDEX idx_threads_list ON threads (account_id, last_date DESC);

CREATE TABLE messages (
    id              INTEGER PRIMARY KEY,
    account_id      INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    dedupe_key      TEXT    NOT NULL,
    message_id_hdr  TEXT,
    thread_id       INTEGER REFERENCES threads (id) ON DELETE SET NULL,
    subject         TEXT,
    from_json       TEXT,
    to_json         TEXT,
    cc_json         TEXT,
    reply_to_json   TEXT,
    date_utc        INTEGER,
    size_bytes      INTEGER,
    snippet         TEXT,
    has_attachments INTEGER NOT NULL DEFAULT 0,
    body_state      TEXT    NOT NULL DEFAULT 'headers' CHECK (body_state IN ('headers', 'full')),
    raw_path        TEXT,
    UNIQUE (account_id, dedupe_key)
);

CREATE INDEX idx_messages_account_date ON messages (account_id, date_utc DESC);
CREATE INDEX idx_messages_thread ON messages (thread_id);

CREATE TABLE message_locations (
    id            INTEGER PRIMARY KEY,
    message_id    INTEGER NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
    mailbox_id    INTEGER NOT NULL REFERENCES mailboxes (id) ON DELETE CASCADE,
    uid           INTEGER NOT NULL,
    seen          INTEGER NOT NULL DEFAULT 0,
    flagged       INTEGER NOT NULL DEFAULT 0,
    answered      INTEGER NOT NULL DEFAULT 0,
    draft         INTEGER NOT NULL DEFAULT 0,
    deleted       INTEGER NOT NULL DEFAULT 0,
    keywords_json TEXT    NOT NULL DEFAULT '[]',
    modseq        INTEGER,
    UNIQUE (mailbox_id, uid)
);

CREATE INDEX idx_locations_message ON message_locations (message_id);

CREATE TABLE message_bodies (
    message_id INTEGER PRIMARY KEY REFERENCES messages (id) ON DELETE CASCADE,
    text_body  TEXT,
    html_clean TEXT,
    fetched_at INTEGER NOT NULL
);

CREATE TABLE attachments (
    id         INTEGER PRIMARY KEY,
    message_id INTEGER NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
    part_id    TEXT    NOT NULL,
    filename   TEXT,
    mime_type  TEXT,
    size_bytes INTEGER,
    content_id TEXT,
    blob_path  TEXT,
    UNIQUE (message_id, part_id)
);

CREATE TABLE tags (
    id         INTEGER PRIMARY KEY,
    account_id INTEGER REFERENCES accounts (id) ON DELETE CASCADE,
    name       TEXT    NOT NULL,
    color      TEXT,
    kind       TEXT    NOT NULL CHECK (kind IN ('label', 'keyword', 'local')),
    server_ref TEXT,
    UNIQUE (account_id, name)
);

CREATE TABLE message_tags (
    message_id INTEGER NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
    tag_id     INTEGER NOT NULL REFERENCES tags (id) ON DELETE CASCADE,
    PRIMARY KEY (message_id, tag_id)
) WITHOUT ROWID;

CREATE INDEX idx_message_tags_tag ON message_tags (tag_id, message_id);

CREATE TABLE outbox (
    id                INTEGER PRIMARY KEY,
    account_id        INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    kind              TEXT    NOT NULL CHECK (kind IN ('send', 'save_draft', 'set_flags', 'move', 'delete', 'tag_add', 'tag_remove')),
    payload_json      TEXT    NOT NULL,
    target_message_id INTEGER,
    target_thread_id  INTEGER,
    state             TEXT    NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'inflight', 'done', 'failed')),
    attempts          INTEGER NOT NULL DEFAULT 0,
    next_attempt_at   INTEGER,
    created_at        INTEGER NOT NULL,
    last_error        TEXT
);

CREATE INDEX idx_outbox_drain ON outbox (state, next_attempt_at);

CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Full-text search (ARCHITECTURE.md §7.3): external-content FTS5 index the
-- writer maintains in the same transaction as message changes (no triggers).
CREATE TABLE messages_fts_content (
    message_id INTEGER PRIMARY KEY,
    subject    TEXT,
    from_text  TEXT,
    to_text    TEXT,
    body       TEXT
);

CREATE VIRTUAL TABLE messages_fts USING fts5(
    subject,
    from_text,
    to_text,
    body,
    content = 'messages_fts_content',
    content_rowid = 'message_id',
    tokenize = 'unicode61 remove_diacritics 2'
);
