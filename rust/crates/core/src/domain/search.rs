//! Full-text search query builder (ARCHITECTURE.md §3.1, §7.3; PRD F7).
//!
//! Parses the search syntax subset — `from:/to:/subject:/in:/tag:/`
//! `is:unread:/is:starred:/has:attachment` plus free text — into one
//! FTS5 `MATCH` expression plus a list of parameterized SQL `WHERE`
//! clauses. Everything from the raw query string is bound as a
//! parameter; nothing is interpolated into SQL.

/// One parsed search request: the FTS5 `MATCH` expression and the
/// account-scoped `WHERE` clauses that refine it.
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    /// The FTS5 `MATCH` expression (prefix-matched free text plus
    /// column-scoped field terms), empty for `WHERE`-only filtering.
    pub match_expr: String,
    /// SQL `WHERE` fragments (without the leading `WHERE`). Only the
    /// `tag:` and `in:` clauses carry one positional `?` each; the `is:`
    /// and `has:` fragments bind nothing.
    pub where_clauses: Vec<String>,
    /// Bound parameter values, pairing 1:1 with the `?`-bearing
    /// [`where_clauses`] in order. Future clause authors must keep
    /// this invariant: every `?` a clause adds needs exactly one param.
    pub params: Vec<String>,
    /// Limit applied to the result page.
    pub limit: u32,
}

/// A single result row from `messages_fts` joined against `messages`.
#[derive(Debug, Clone)]
pub struct MessageHit {
    pub message_id: i64,
    pub account_id: i64,
    pub subject: Option<String>,
    pub snippet: Option<String>,
}

/// Parses `query` into [`SearchQuery`].
///
/// Free text is split on whitespace; each term becomes a quoted prefix
/// expression (`'term*'`). Phrases in double quotes become a single
/// quoted expression. Field prefixes map to FTS5 column-scoped
/// terms:
///   - `from:`/`to:`/`subject:` → `from_text:`/`to_text:`/`subject:` in
///     `match_expr`
///   - `tag:` → a `WHERE` clause on `message_tags`/`tags`
///   - `in:` → a `WHERE` clause on `message_locations`/`mailboxes` `kind`
///   - `is:unread` → `seen = 0` on `message_locations`
///   - `is:starred` → `flagged = 1` on `message_locations`
///   - `has:attachment` → `has_attachments = 1` on `messages`
///
/// An empty query returns the default: match everything, no `WHERE`.
pub fn parse(query: &str) -> Result<SearchQuery, SearchError> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(SearchQuery {
            match_expr: String::new(),
            where_clauses: Vec::new(),
            params: Vec::new(),
            limit: 50,
        });
    }

    let tokens = tokenize(trimmed)?;

    let mut clauses: Vec<String> = Vec::new();
    let mut params: Vec<String> = Vec::new();
    let mut match_terms: Vec<String> = Vec::new();
    for token in &tokens {
        // Split `field:value` on the first `:`; free text has no `:`.
        let (field, value) = match token.find(':') {
            Some(colon) if colon > 0 => (&token[..colon], &token[colon + 1..]),
            _ => ("", token.as_str()),
        };
        if field.is_empty() {
            // A trailing colon on free text (e.g. `:` alone) is not a field.
            if token.ends_with(':') {
                return Err(SearchError::UnknownField(token.clone()));
            }
            match_terms.push(format!("{}*", escape_fts(token)));
            continue;
        }
        // Known fields consume their value; anything else is an error.
        let Some(value) = (!value.is_empty()).then_some(value) else {
            return Err(SearchError::MissingArgument(field.to_string()));
        };
        match field {
            "from" => {
                match_terms.push(format!("from_text:{}", escape_fts(value)));
            }
            "to" => {
                match_terms.push(format!("to_text:{}", escape_fts(value)));
            }
            "subject" => {
                match_terms.push(format!("subject:{}", escape_fts(value)));
            }
            "tag" => {
                clauses.push(
                    "EXISTS (SELECT 1 FROM message_tags mt JOIN tags t ON t.id = mt.tag_id \
                     WHERE mt.message_id = m.id AND t.name = ?)"
                        .to_string(),
                );
                params.push(value.to_string());
            }
            "in" => {
                clauses.push(
                    "EXISTS (SELECT 1 FROM message_locations ml JOIN mailboxes mb \
                     ON mb.id = ml.mailbox_id WHERE ml.message_id = m.id AND mb.kind = ?)"
                        .to_string(),
                );
                params.push(value.to_string());
            }
            "is" => match value {
                "unread" => clauses.push(
                    "EXISTS (SELECT 1 FROM message_locations ml \
                     WHERE ml.message_id = m.id AND ml.seen = 0 AND NOT ml.deleted)"
                        .to_string(),
                ),
                "starred" => clauses.push(
                    "EXISTS (SELECT 1 FROM message_locations ml \
                     WHERE ml.message_id = m.id AND ml.flagged = 1 AND NOT ml.deleted)"
                        .to_string(),
                ),
                other => return Err(SearchError::UnknownField(format!("is:{other}"))),
            },
            "has" => match value {
                "attachment" => clauses.push("m.has_attachments = 1".to_string()),
                other => return Err(SearchError::UnknownField(format!("has:{other}"))),
            },
            _ => return Err(SearchError::UnknownField(format!("{field}:{value}"))),
        }
    }

    let match_expr = match_terms.join(" ");

    Ok(SearchQuery {
        match_expr,
        where_clauses: clauses,
        params,
        limit: 50,
    })
}

/// `account_id` value that searches every account instead of one.
pub const ALL_ACCOUNTS: i64 = -1;

/// The `SELECT ... FROM messages_fts JOIN messages m` prologue the
/// search runs against ([`search`] appends `WHERE ... ORDER BY ...
/// LIMIT ?`; parameters are bound positionally via
/// [`rusqlite::params_from_iter`]).
///
/// Kept `pub` for the upcoming snippet and result-count extension;
/// [`search`] is its only caller today.
pub fn search_sql_base() -> &'static str {
    "SELECT m.id, m.account_id, m.subject, NULL as snippet \
     FROM messages_fts \
     JOIN messages m ON m.id = messages_fts.rowid"
}

/// Runs [`parse`] and executes the search against `conn`. The
/// `account_id` scopes results to one account, or
/// [`ALL_ACCOUNTS`] searches every account.
///
/// The `WHERE` conditions — account scope, `where_clauses`, and the
/// FTS `MATCH` — are all joined as top-level `AND` conjunctions:
/// SQLite only admits `MATCH` as a direct conjunct of the fts table
/// query, not inside an `OR` branch.
pub fn search(
    conn: &rusqlite::Connection,
    account_id: i64,
    query: &str,
) -> Result<Vec<MessageHit>, SearchError> {
    let SearchQuery {
        match_expr,
        where_clauses,
        params,
        limit,
    } = parse(query)?;

    // Invariant from the SearchQuery docs: every `?` carried by a
    // where clause must have exactly one bound param, in order.
    debug_assert_eq!(
        where_clauses
            .iter()
            .map(|c| c.matches('?').count())
            .sum::<usize>(),
        params.len()
    );

    let mut sql = search_sql_base().to_string();

    let mut conds: Vec<String> = Vec::with_capacity(2 + where_clauses.len());
    let mut binds: Vec<Bind> = Vec::with_capacity(2 + where_clauses.len());
    if account_id != ALL_ACCOUNTS {
        conds.push("m.account_id = ?".to_string());
        binds.push(Bind::I64(account_id));
    }
    conds.extend(where_clauses.iter().cloned());
    for p in &params {
        binds.push(Bind::Str(p.clone()));
    }
    if !match_expr.is_empty() {
        conds.push("messages_fts MATCH ?".to_string());
        binds.push(Bind::Str(match_expr.clone()));
    }
    if !conds.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conds.join(" AND "));
    }
    sql.push_str(" ORDER BY bm25(messages_fts, 0, 5.0, 2.0, 1.0, 1.0) LIMIT ?");
    binds.push(Bind::I64(limit as i64));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(binds.iter()), |row| {
        Ok(MessageHit {
            message_id: row.get(0)?,
            account_id: row.get(1)?,
            subject: row.get(2)?,
            snippet: row.get(3)?,
        })
    })?;

    let mut hits: Vec<MessageHit> = Vec::new();
    for hit in rows {
        hits.push(hit?);
    }

    Ok(hits)
}

/// Splits `input` into whitespace-separated tokens, honoring double
/// quotes for phrases. Returns [`SearchError::UnterminatedPhrase`] when
/// an opening quote has no matching close.
fn tokenize(input: &str) -> Result<Vec<String>, SearchError> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens: Vec<String> = Vec::new();
    let mut pos = 0;
    while pos < chars.len() {
        // Skip whitespace.
        while pos < chars.len() && chars[pos].is_whitespace() {
            pos += 1;
        }
        if pos >= chars.len() {
            break;
        }
        if chars[pos] == '"' {
            pos += 1; // consume opening quote
            let mut buf = String::new();
            let mut closed = false;
            while pos < chars.len() {
                if chars[pos] == '"' {
                    pos += 1;
                    closed = true;
                    break;
                }
                buf.push(chars[pos]);
                pos += 1;
            }
            if !closed {
                return Err(SearchError::UnterminatedPhrase);
            }
            tokens.push(buf);
        } else {
            let mut buf = String::new();
            while pos < chars.len() && !chars[pos].is_whitespace() && chars[pos] != '"' {
                buf.push(chars[pos]);
                pos += 1;
            }
            tokens.push(buf);
        }
    }
    Ok(tokens)
}

/// Escapes FTS5 special characters so a user value cannot change the
/// shape of a `MATCH` expression. Values containing special characters
/// are wrapped in double quotes; internal quotes are doubled. Bare
/// alphanumeric values pass through unchanged.
fn escape_fts(value: &str) -> String {
    const SPECIAL: &[char] = &[
        ':', ' ', '(', ')', '"', '*', '^', '-', '+', '<', '>', '~', '/', '\\', '[', ']', '{', '}',
        '|',
    ];
    if !value.chars().any(|c| SPECIAL.contains(&c)) {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        if c == '"' {
            out.push('"');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// A dynamically-bound query parameter: unifies `i64` and `String`
/// values so [`rusqlite::params_from_iter`] can bind the mixed vector
/// positionally against the `?` placeholders.
enum Bind {
    Str(String),
    I64(i64),
}

impl rusqlite::types::ToSql for Bind {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>, rusqlite::Error> {
        match self {
            Bind::Str(s) => Ok(rusqlite::types::ToSqlOutput::Owned(
                rusqlite::types::Value::Text(s.clone()),
            )),
            Bind::I64(n) => Ok(rusqlite::types::ToSqlOutput::Owned(
                rusqlite::types::Value::Integer(*n),
            )),
        }
    }
}

/// Errors while parsing or running a search.
#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error("unknown search field: {0}")]
    UnknownField(String),
    #[error("missing argument for field: {0}")]
    MissingArgument(String),
    #[error("unterminated quoted phrase")]
    UnterminatedPhrase,
    #[error("sqlite: {0}")]
    Storage(#[from] rusqlite::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fixture DDL is hand-copied from
    /// `rust/crates/core/src/infra/db/migrations/0001_init.sql:158-174`
    /// — keep the shapes in sync whenever that migration changes.
    fn open_fixture() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE messages (
                id INTEGER PRIMARY KEY,
                account_id INTEGER,
                subject TEXT,
                has_attachments INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE mailboxes (
                id INTEGER PRIMARY KEY,
                account_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                kind TEXT NOT NULL
            );
            CREATE TABLE message_locations (
                id INTEGER PRIMARY KEY,
                message_id INTEGER NOT NULL,
                mailbox_id INTEGER NOT NULL,
                uid INTEGER NOT NULL,
                seen INTEGER NOT NULL DEFAULT 0,
                flagged INTEGER NOT NULL DEFAULT 0,
                deleted INTEGER NOT NULL DEFAULT 0,
                UNIQUE (mailbox_id, uid)
            );
            CREATE TABLE tags (
                id INTEGER PRIMARY KEY,
                account_id INTEGER,
                name TEXT NOT NULL,
                kind TEXT NOT NULL
            );
            CREATE TABLE message_tags (
                message_id INTEGER NOT NULL,
                tag_id INTEGER NOT NULL,
                PRIMARY KEY (message_id, tag_id)
            );
            CREATE TABLE messages_fts_content (
                message_id INTEGER PRIMARY KEY,
                subject TEXT,
                from_text TEXT,
                to_text TEXT,
                body TEXT
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
            INSERT INTO messages (id, account_id, subject, has_attachments)
                 VALUES (1, 42, 'Quarterly report', 1);
            INSERT INTO messages (id, account_id, subject, has_attachments)
                 VALUES (2, 7, 'Team budgets', 0);
            INSERT INTO mailboxes VALUES (1, 42, 'Inbox', 'inbox');
            INSERT INTO mailboxes VALUES (2, 7, 'Archive', 'archive');
            INSERT INTO message_locations
                 (id, message_id, mailbox_id, uid, seen, flagged, deleted)
                 VALUES (1, 1, 1, 1, 1, 0, 0);
            INSERT INTO message_locations
                 (id, message_id, mailbox_id, uid, seen, flagged, deleted)
                 VALUES (2, 2, 2, 1, 0, 1, 0);
            INSERT INTO tags VALUES (1, 42, 'invoices', 'local');
            INSERT INTO message_tags VALUES (1, 1);
            INSERT INTO messages_fts_content (message_id, subject, from_text, to_text, body)
                 VALUES (1, 'Quarterly report', 'ana', 'bo', 'the budgets look fine');
            INSERT INTO messages_fts_content (message_id, subject, from_text, to_text, body)
                 VALUES (2, 'Team budgets', 'cy', 'lin', 'the budget ramp');
            INSERT INTO messages_fts (rowid, subject, from_text, to_text, body)
                 VALUES (1, 'Quarterly report', 'ana', 'bo', 'the budgets look fine');
            INSERT INTO messages_fts (rowid, subject, from_text, to_text, body)
                 VALUES (2, 'Team budgets', 'cy', 'lin', 'the budget ramp');",
        )
        .unwrap();
        conn
    }

    #[test]
    fn search_scopes_by_account_and_matches_free_text() {
        let conn = open_fixture();

        let hits = search(&conn, 42, "budgets").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message_id, 1);
        assert_eq!(hits[0].account_id, 42);
        assert_eq!(hits[0].subject.as_deref(), Some("Quarterly report"));
        assert!(hits[0].snippet.is_none());

        // -1 searches every account. The literal (not ALL_ACCOUNTS)
        // pins the IPC sentinel: the Dart client sends -1 verbatim,
        // so a drifted const value fails this test.
        let all = search(&conn, -1, "budgets").unwrap();
        assert_eq!(all.len(), 2);

        // Non-sentinel negatives are empty scopes, not all-accounts.
        assert!(search(&conn, -5, "budgets").unwrap().is_empty());
        assert!(search(&conn, -2, "budgets").unwrap().is_empty());
    }

    #[test]
    fn tag_clause_hits_only_tagged_message() {
        let conn = open_fixture();

        let hits = search(&conn, -1, "tag:invoices").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message_id, 1);
    }

    #[test]
    fn clause_only_search_keeps_the_account_scope() {
        let conn = open_fixture();

        // Bind shape [account, clause param, limit] — no MATCH.
        let hits = search(&conn, 42, "tag:invoices").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message_id, 1);

        // The tag lives only on the account-42 message.
        assert!(search(&conn, 7, "tag:invoices").unwrap().is_empty());
    }

    #[test]
    fn multi_clause_search_without_account_runs_clause_only_binds() {
        let conn = open_fixture();

        let inbox = search(&conn, -1, "tag:invoices in:inbox").unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].message_id, 1);

        let archive = search(&conn, -1, "in:archive is:starred").unwrap();
        assert_eq!(archive.len(), 1);
        assert_eq!(archive[0].message_id, 2);
    }

    #[test]
    fn parse_errors_propagate_through_search() {
        let conn = open_fixture();

        let unknown = search(&conn, ALL_ACCOUNTS, "cc:x").unwrap_err();
        assert!(matches!(unknown, SearchError::UnknownField(_)));

        let unterminated = search(&conn, 42, "\"unterminated").unwrap_err();
        assert!(matches!(unterminated, SearchError::UnterminatedPhrase));
    }

    #[test]
    fn is_unread_clause_hits_only_unseen_message() {
        let conn = open_fixture();

        let hits = search(&conn, -1, "is:unread").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message_id, 2);
    }

    #[test]
    fn is_starred_clause_hits_only_flagged_message() {
        let conn = open_fixture();

        let hits = search(&conn, -1, "is:starred").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message_id, 2);
    }

    #[test]
    fn deleted_locations_are_excluded_from_flag_clauses() {
        let conn = open_fixture();

        // A trashed message that would otherwise match both flag
        // clauses (unseen and flagged) — its location row is marked
        // deleted, so the NOT ml.deleted filter must keep it hidden.
        conn.execute(
            "INSERT INTO messages (id, account_id, subject, has_attachments)
             VALUES (3, 7, 'Ignore me', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_locations
                (id, message_id, mailbox_id, uid, seen, flagged, deleted)
             VALUES (3, 3, 2, 2, 0, 1, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages_fts_content
                (message_id, subject, from_text, to_text, body)
             VALUES (3, 'Ignore me', 'x', 'x', 'nothing to see')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages_fts (rowid, subject, from_text, to_text, body)
             VALUES (3, 'Ignore me', 'x', 'x', 'nothing to see')",
            [],
        )
        .unwrap();

        let unread = search(&conn, -1, "is:unread").unwrap();
        assert_eq!(unread.len(), 1);
        assert_eq!(unread[0].message_id, 2);

        let starred = search(&conn, -1, "is:starred").unwrap();
        assert_eq!(starred.len(), 1);
        assert_eq!(starred[0].message_id, 2);
    }

    #[test]
    fn has_attachment_clause_hits_only_attachment_message() {
        let conn = open_fixture();

        let hits = search(&conn, -1, "has:attachment").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message_id, 1);
    }

    #[test]
    fn in_clause_hits_only_that_mailbox_kind() {
        let conn = open_fixture();

        let hits = search(&conn, -1, "in:inbox").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message_id, 1);
    }

    #[test]
    fn empty_query_without_account_runs_without_where() {
        let conn = open_fixture();

        let hits = search(&conn, -1, "").unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().all(|hit| hit.snippet.is_none()));
    }

    #[test]
    fn limit_truncates_the_result_page() {
        let conn = open_fixture();
        for id in 3..58 {
            conn.execute(
                "INSERT INTO messages (id, account_id, subject, has_attachments)
                 VALUES (?1, 42, ?2, 0)",
                rusqlite::params![id, format!("filler {id}")],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO messages_fts_content
                    (message_id, subject, from_text, to_text, body)
                 VALUES (?1, ?2, 'x', 'x', 'filler body')",
                rusqlite::params![id, format!("filler {id}")],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO messages_fts (rowid, subject, from_text, to_text, body)
                 VALUES (?1, ?2, 'x', 'x', 'filler body')",
                rusqlite::params![id, format!("filler {id}")],
            )
            .unwrap();
        }

        let hits = search(&conn, -1, "").unwrap();
        assert_eq!(hits.len(), 50);
    }

    #[test]
    fn combined_free_text_and_tag_clause_hit_both_bind() {
        let conn = open_fixture();

        // Exercises the full bind vector in one query: the account
        // i64, the clause param, the MATCH expression, and the limit,
        // in that positional order.
        let hits = search(&conn, 42, "budgets tag:invoices").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message_id, 1);
        assert_eq!(hits[0].account_id, 42);
    }

    #[test]
    fn combined_free_text_and_flag_clause_can_be_empty() {
        let conn = open_fixture();

        // Message 1 (the only account-42 match for "budgets") is
        // seen, so is:unread filters it out.
        let hits = search(&conn, 42, "budgets is:unread").unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn multi_clause_bind_order_survives_three_params() {
        let conn = open_fixture();

        // Two clause params (tag, in) interleaved with the MATCH and
        // limit binds: the positional vector is account i64, tag
        // param, in param, MATCH, limit. A single drift in that order
        // misaligns every later bind.
        let hits = search(&conn, 42, "budgets tag:invoices in:inbox").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message_id, 1);
        assert_eq!(hits[0].account_id, 42);

        // The missing tag drops the row without shifting binds.
        let none = search(&conn, 42, "budgets tag:missing in:inbox").unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn account_scoped_empty_query_keeps_the_account_cond() {
        let conn = open_fixture();

        // Single-condition WHERE (account only), no MATCH: the scope
        // still applies to the match-everything page.
        let hits = search(&conn, 42, "").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message_id, 1);
    }

    #[test]
    fn prepare_error_propagates_as_storage() {
        // No tables at all: prepare must fail and surface as Storage
        // instead of panicking.
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let err = search(&conn, ALL_ACCOUNTS, "budgets").unwrap_err();
        assert!(matches!(err, SearchError::Storage(_)));
    }

    #[test]
    fn row_map_error_propagates_as_storage() {
        let conn = open_fixture();

        // A row whose account_id cannot map to i64 forces the
        // row-mapping error path (`hits.push(hit?)`). The TEXT
        // account_id is FK-impossible under the production NOT NULL
        // integer schema; in-memory dynamic typing keeps it, which
        // is exactly what forces the row-map failure.
        conn.execute(
            "INSERT INTO messages (id, account_id, subject, has_attachments)
             VALUES (3, 'not-a-number', 'Text budgets', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages_fts_content
                (message_id, subject, from_text, to_text, body)
             VALUES (3, 'Text budgets', 'x', 'x', 'budgets again')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages_fts (rowid, subject, from_text, to_text, body)
             VALUES (3, 'Text budgets', 'x', 'x', 'budgets again')",
            [],
        )
        .unwrap();

        let err = search(&conn, ALL_ACCOUNTS, "budgets").unwrap_err();
        assert!(matches!(err, SearchError::Storage(_)));
    }

    #[test]
    fn empty_query_matches_everything() {
        let q = parse("").unwrap();
        assert_eq!(q.match_expr, "");
        assert!(q.where_clauses.is_empty());
        assert_eq!(q.limit, 50);
    }

    #[test]
    fn free_text_becomes_prefix_terms() {
        let q = parse("hello world").unwrap();
        assert_eq!(q.match_expr, "hello* world*");
        assert!(q.where_clauses.is_empty());
    }

    #[test]
    fn quoted_phrase_is_one_term() {
        let q = parse("\"budgets look fine\"").unwrap();
        assert_eq!(q.match_expr, "\"budgets look fine\"*");
    }

    #[test]
    fn field_prefixes_map_to_match_terms() {
        let q = parse("from:ana to:bo subject:report").unwrap();
        assert_eq!(q.match_expr, "from_text:ana to_text:bo subject:report");
        assert!(q.where_clauses.is_empty());
    }

    #[test]
    fn tag_in_is_flags_are_where_clauses() {
        let q = parse("tag:invoices in:inbox is:unread is:starred has:attachment").unwrap();
        assert_eq!(q.match_expr, "");
        assert_eq!(q.where_clauses.len(), 5);
        assert_eq!(q.params.len(), 2);
    }

    #[test]
    fn free_text_with_fields_both_apply() {
        let q = parse("budgets from:ana").unwrap();
        assert_eq!(q.match_expr, "budgets* from_text:ana");
        assert_eq!(q.where_clauses.len(), 0);
    }

    #[test]
    fn unknown_field_is_an_error() {
        let err = parse("cc:ana").unwrap_err();
        assert!(matches!(err, SearchError::UnknownField(_)));
    }

    #[test]
    fn missing_argument_is_an_error() {
        let err = parse("from:").unwrap_err();
        assert!(matches!(err, SearchError::MissingArgument(_)));
    }

    #[test]
    fn unterminated_phrase_is_an_error() {
        let err = parse("\"unclosed").unwrap_err();
        assert!(matches!(err, SearchError::UnterminatedPhrase));
    }

    #[test]
    fn fts_special_chars_are_escaped() {
        let q = parse("from:john.doe+spam@example.com").unwrap();
        assert_eq!(q.match_expr, "from_text:\"john.doe+spam@example.com\"");
    }

    #[test]
    fn phrase_with_quotes_strips_them() {
        let q = parse("\"hello world\" from:boss").unwrap();
        assert_eq!(q.match_expr, "\"hello world\"* from_text:boss");
    }
}
