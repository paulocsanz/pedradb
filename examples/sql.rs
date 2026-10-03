//! A tiny SQL dialect over the same ordered keys.
//!
//! Tables are prefixes. This is not Postgres — it is the smallest layer that
//! still looks like SQL, so you can see how a query engine sits on Pedra
//! without standing up a server.
//!
//! ```sh
//! cargo run -p pedradb-examples --example sql
//! ```

use pedradb_sql::{QueryResult, SqlEngine};

fn scratch(name: &str) -> std::path::PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("pedradb-ex-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn run() -> pedradb_sql::Result<()> {
    let dir = scratch("sql");
    let mut sql = SqlEngine::open(&dir)?;

    sql.execute("CREATE TABLE users")?;
    sql.execute("INSERT INTO users VALUES ('ada', 'lovelace')")?;
    sql.execute("INSERT INTO users VALUES ('bob', 'builder')")?;

    match sql.execute("SELECT * FROM users WHERE key = 'ada'")? {
        QueryResult::Rows(rows) => {
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].key, b"ada");
            assert_eq!(rows[0].value, b"lovelace");
        }
        other => panic!("expected rows, got {other:?}"),
    }

    match sql.execute("SELECT * FROM users")? {
        QueryResult::Rows(rows) => assert_eq!(rows.len(), 2),
        other => panic!("expected rows, got {other:?}"),
    }

    match sql.execute("DELETE FROM users WHERE key = 'bob'")? {
        QueryResult::Ok { rows_affected } => assert_eq!(rows_affected, 1),
        other => panic!("expected ok, got {other:?}"),
    }

    println!("sql: users.ada=lovelace; bob deleted; SELECT * has 1 row");
    sql.close()?;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

fn main() -> pedradb_sql::Result<()> {
    run()
}

#[cfg(test)]
mod tests {
    #[test]
    fn smoke() {
        super::run().expect("sql");
    }
}
