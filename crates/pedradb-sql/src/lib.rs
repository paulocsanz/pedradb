//! Minimal SQL subset over PedraDB (RFC-0010 P2.2).
//!
//! Supports a tiny wire-free dialect for embedding / demos:
//!
//! ```text
//! CREATE TABLE t
//! INSERT INTO t VALUES ('k', 'v')
//! SELECT * FROM t WHERE key = 'k'
//! SELECT * FROM t
//! DELETE FROM t WHERE key = 'k'
//! ```
//!
//! Tables are key prefixes: row keys are `t/{user_key}`. Not Postgres-compatible.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use pedradb_core::{Db, OpenOptions, Result as CoreResult};
use thiserror::Error;

/// SQL execution errors.
#[derive(Debug, Error)]
pub enum SqlError {
    /// Engine I/O.
    #[error("pedradb: {0}")]
    Core(#[from] pedradb_core::CoreError),
    /// Parse / unsupported statement.
    #[error("sql: {0}")]
    Parse(String),
    /// Runtime (missing table, etc.).
    #[error("exec: {0}")]
    Exec(String),
}

/// Result alias.
pub type Result<T> = std::result::Result<T, SqlError>;

/// One result row: key + value as UTF-8 lossy strings for display, plus raw bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// User key (without table prefix).
    pub key: Vec<u8>,
    /// Value bytes.
    pub value: Vec<u8>,
}

/// Outcome of executing one statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryResult {
    /// DDL / DML without row set.
    Ok {
        /// Rows affected (inserts/deletes).
        rows_affected: u64,
    },
    /// SELECT rows.
    Rows(Vec<Row>),
}

/// SQL session over one PedraDB directory.
pub struct SqlEngine {
    db: Db,
}

impl SqlEngine {
    /// Open engine at path.
    ///
    /// # Errors
    /// PedraDB open.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let db = Db::open_with(
            path,
            OpenOptions {
                wal_recovery: Default::default(),
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
            },
        )?;
        Ok(Self { db })
    }

    /// Execute one statement (trimmed; single statement only).
    ///
    /// # Errors
    /// Parse or exec failures.
    pub fn execute(&mut self, sql: &str) -> Result<QueryResult> {
        let s = sql.trim().trim_end_matches(';').trim();
        let upper = s.to_ascii_uppercase();
        if upper.starts_with("CREATE TABLE ") {
            return self.exec_create(s);
        }
        if upper.starts_with("INSERT INTO ") {
            return self.exec_insert(s);
        }
        if upper.starts_with("SELECT ") {
            return self.exec_select(s);
        }
        if upper.starts_with("DELETE FROM ") {
            return self.exec_delete(s);
        }
        Err(SqlError::Parse(format!("unsupported: {s}")))
    }

    fn push_len(buf: &mut Vec<u8>, part: &[u8]) {
        let n = u32::try_from(part.len()).expect("len fits u32");
        buf.extend_from_slice(&n.to_be_bytes());
        buf.extend_from_slice(part);
    }

    fn table_marker(table: &str) -> Vec<u8> {
        // F71: not `sql/table/{name}` (prefix of longer names).
        let mut k = b"sql/t".to_vec();
        Self::push_len(&mut k, table.as_bytes());
        k
    }

    fn row_key(table: &str, user: &[u8]) -> Vec<u8> {
        // F71: not `sql/row/{table}/{user}` slash join.
        let mut k = b"sql/r".to_vec();
        Self::push_len(&mut k, table.as_bytes());
        Self::push_len(&mut k, user);
        k
    }

    fn row_prefix(table: &str) -> Vec<u8> {
        let mut k = b"sql/r".to_vec();
        Self::push_len(&mut k, table.as_bytes());
        k
    }

    fn exec_create(&mut self, s: &str) -> Result<QueryResult> {
        // CREATE TABLE name
        let rest = s[12..].trim();
        let table = rest
            .split_whitespace()
            .next()
            .ok_or_else(|| SqlError::Parse("CREATE TABLE needs name".into()))?;
        if !is_ident(table) {
            return Err(SqlError::Parse(format!("bad table name {table}")));
        }
        let marker = Self::table_marker(table);
        if self.db.get(&marker).is_some() {
            return Err(SqlError::Exec(format!("table {table} exists")));
        }
        self.db.put(&marker, b"1")?;
        Ok(QueryResult::Ok { rows_affected: 0 })
    }

    fn exec_insert(&mut self, s: &str) -> Result<QueryResult> {
        // INSERT INTO t VALUES ('k', 'v')
        let rest = &s[11..];
        let rest = rest.trim();
        let (table, after_table) = split_ident(rest)?;
        let after_table = after_table.trim();
        let upper = after_table.to_ascii_uppercase();
        if !upper.starts_with("VALUES") {
            return Err(SqlError::Parse("expected VALUES".into()));
        }
        let vals = after_table[6..].trim();
        let (k, v) = parse_two_strings(vals)?;
        self.require_table(table)?;
        self.db
            .put(Self::row_key(table, k.as_bytes()), v.as_bytes())?;
        Ok(QueryResult::Ok { rows_affected: 1 })
    }

    fn exec_select(&mut self, s: &str) -> Result<QueryResult> {
        // SELECT * FROM t [WHERE key = 'k']
        let rest = &s[6..];
        let rest = rest.trim();
        if !rest.to_ascii_uppercase().starts_with("* FROM ") {
            return Err(SqlError::Parse("only SELECT * FROM supported".into()));
        }
        let rest = rest[7..].trim();
        let (table, after) = split_ident(rest)?;
        self.require_table(table)?;
        let after = after.trim();
        let prefix = Self::row_prefix(table);
        if after.is_empty() {
            let rows = self.scan_table(&prefix);
            return Ok(QueryResult::Rows(rows));
        }
        let u = after.to_ascii_uppercase();
        if !u.starts_with("WHERE KEY = ") && !u.starts_with("WHERE KEY=") {
            return Err(SqlError::Parse("only WHERE key = '...' supported".into()));
        }
        let eq = after
            .find('=')
            .ok_or_else(|| SqlError::Parse("missing =".into()))?;
        let lit = after[eq + 1..].trim();
        let key = parse_string_lit(lit)?;
        match self.db.get(&Self::row_key(table, key.as_bytes())) {
            Some(val) => Ok(QueryResult::Rows(vec![Row {
                key: key.into_bytes(),
                value: val.to_vec(),
            }])),
            None => Ok(QueryResult::Rows(vec![])),
        }
    }

    fn exec_delete(&mut self, s: &str) -> Result<QueryResult> {
        // DELETE FROM t WHERE key = 'k'
        let rest = s[11..].trim();
        let (table, after) = split_ident(rest)?;
        self.require_table(table)?;
        let after = after.trim();
        let u = after.to_ascii_uppercase();
        if !u.starts_with("WHERE KEY") {
            return Err(SqlError::Parse("DELETE requires WHERE key =".into()));
        }
        let eq = after
            .find('=')
            .ok_or_else(|| SqlError::Parse("missing =".into()))?;
        let key = parse_string_lit(after[eq + 1..].trim())?;
        let rk = Self::row_key(table, key.as_bytes());
        if self.db.get(&rk).is_none() {
            return Ok(QueryResult::Ok { rows_affected: 0 });
        }
        self.db.delete(&rk)?;
        Ok(QueryResult::Ok { rows_affected: 1 })
    }

    fn require_table(&self, table: &str) -> Result<()> {
        if self.db.get(&Self::table_marker(table)).is_none() {
            return Err(SqlError::Exec(format!("no such table: {table}")));
        }
        Ok(())
    }

    fn scan_table(&self, prefix: &[u8]) -> Vec<Row> {
        // Exclusive end: after length-prefixed table component, next byte 0x00..0xff
        // of user-len field. Using prefix_successor on the table prefix is wrong for
        // length-prefix encoding — children are prefix||u32be(user)||user, so end is
        // table_prefix with last length nibble... Use unbounded end after a max
        // sentinel: table_prefix + 0xff,0xff,0xff,0xff (len=u32::MAX is impossible).
        let mut end = prefix.to_vec();
        end.extend_from_slice(&u32::MAX.to_be_bytes());
        let pairs = self.db.range_limited(
            std::ops::Bound::Included(prefix),
            std::ops::Bound::Excluded(end.as_slice()),
            None,
        );
        let plen = prefix.len();
        pairs
            .into_iter()
            .filter_map(|(k, v)| {
                let rest = k.get(plen..)?;
                if rest.len() < 4 {
                    return None;
                }
                let n = u32::from_be_bytes(rest[0..4].try_into().ok()?) as usize;
                if rest.len() != 4 + n {
                    return None;
                }
                Some(Row {
                    key: rest[4..].to_vec(),
                    value: v.to_vec(),
                })
            })
            .collect()
    }

    /// Close DB.
    ///
    /// # Errors
    /// WAL close.
    pub fn close(self) -> CoreResult<()> {
        self.db.close()
    }
}

fn is_ident(s: &str) -> bool {
    let mut c = s.chars();
    match c.next() {
        Some(ch) if ch.is_ascii_alphabetic() || ch == '_' => {}
        _ => return false,
    }
    c.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn split_ident(s: &str) -> Result<(&str, &str)> {
    let s = s.trim_start();
    let end = s
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(s.len());
    if end == 0 {
        return Err(SqlError::Parse("expected identifier".into()));
    }
    let id = &s[..end];
    if !is_ident(id) {
        return Err(SqlError::Parse(format!("bad ident {id}")));
    }
    Ok((id, &s[end..]))
}

fn parse_string_lit(s: &str) -> Result<String> {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('\'') && s.ends_with('\'') {
        return Ok(s[1..s.len() - 1].replace("''", "'"));
    }
    Err(SqlError::Parse(format!("expected string lit, got {s}")))
}

fn parse_two_strings(s: &str) -> Result<(String, String)> {
    let s = s.trim();
    if !s.starts_with('(') || !s.ends_with(')') {
        return Err(SqlError::Parse("expected (k, v)".into()));
    }
    let inner = &s[1..s.len() - 1];
    let mut parts = split_csv_lits(inner)?;
    if parts.len() != 2 {
        return Err(SqlError::Parse("VALUES needs two strings".into()));
    }
    Ok((parts.remove(0), parts.remove(0)))
}

fn split_csv_lits(s: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_str = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if in_str {
            if c == '\'' {
                if chars.peek() == Some(&'\'') {
                    cur.push('\'');
                    chars.next();
                } else {
                    in_str = false;
                    out.push(std::mem::take(&mut cur));
                }
            } else {
                cur.push(c);
            }
        } else if c == '\'' {
            in_str = true;
        } else if c == ',' || c.is_whitespace() {
            continue;
        } else {
            return Err(SqlError::Parse("bad VALUES list".into()));
        }
    }
    if in_str {
        return Err(SqlError::Parse("unterminated string".into()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp() -> std::path::PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let i = N.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("pedradb-sql-{n}-{i}"));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn create_insert_select_delete() {
        let dir = temp();
        let mut eng = SqlEngine::open(&dir).unwrap();
        eng.execute("CREATE TABLE users").unwrap();
        eng.execute("INSERT INTO users VALUES ('1', 'ada')")
            .unwrap();
        eng.execute("INSERT INTO users VALUES ('2', 'bob')")
            .unwrap();
        match eng.execute("SELECT * FROM users WHERE key = '1'").unwrap() {
            QueryResult::Rows(rows) => {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].value, b"ada");
            }
            other => panic!("{other:?}"),
        }
        match eng.execute("SELECT * FROM users").unwrap() {
            QueryResult::Rows(rows) => assert_eq!(rows.len(), 2),
            other => panic!("{other:?}"),
        }
        eng.execute("DELETE FROM users WHERE key = '1'").unwrap();
        match eng.execute("SELECT * FROM users WHERE key = '1'").unwrap() {
            QueryResult::Rows(rows) => assert!(rows.is_empty()),
            other => panic!("{other:?}"),
        }
        eng.close().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
