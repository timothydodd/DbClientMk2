//! SQLite through rusqlite. Work runs on a blocking thread; cancellation calls `interrupt()` on
//! the connection, which makes the running statement fail with an interrupt error.

use crate::cell::format_bytes;
use crate::{elapsed_ms, CancellationToken, DataError, DbProvider, Result};
use dbclient_core::models::*;
use dbclient_core::sql_identifier::{quote, SqlDialect};
use rusqlite::fallible_iterator::FallibleIterator;
use rusqlite::types::ValueRef;
use std::time::Duration;

pub struct SqliteProvider {
    config: ConnectionConfig,
}

impl SqliteProvider {
    pub fn new(config: ConnectionConfig) -> Self {
        Self { config }
    }

    fn open(&self) -> Result<rusqlite::Connection> {
        if self.config.file_name.trim().is_empty() {
            return Err(DataError::Config("No SQLite file selected.".into()));
        }
        let conn = rusqlite::Connection::open(&self.config.file_name).map_err(|e| DataError::Connect(e.to_string()))?;
        conn.busy_timeout(Duration::from_secs(self.config.command_timeout.max(1) as u64)).map_err(|e| DataError::Connect(e.to_string()))?;
        Ok(conn)
    }

    fn display_name(&self) -> String {
        std::path::Path::new(&self.config.file_name).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()
    }
}

fn format_value(v: ValueRef<'_>) -> Option<String> {
    match v {
        ValueRef::Null => None,
        ValueRef::Integer(i) => Some(i.to_string()),
        ValueRef::Real(f) => Some(crate::cell::format_f64(f)),
        ValueRef::Text(t) => Some(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Some(format_bytes(b)),
    }
}

fn load_columns(conn: &rusqlite::Connection, table: &str) -> rusqlite::Result<Vec<DbColumn>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", quote(SqlDialect::Sqlite, table)))?;
    let rows = stmt.query_map([], |r| {
        Ok(DbColumn {
            name: r.get::<_, String>("name")?,
            data_type: r.get::<_, String>("type")?,
            is_primary_key: r.get::<_, i64>("pk")? > 0,
            is_nullable: r.get::<_, i64>("notnull")? != 1,
        })
    })?;
    rows.collect()
}

/// `PRAGMA foreign_key_list` rows grouped by constraint id.
fn load_foreign_keys(conn: &rusqlite::Connection, table: &str) -> rusqlite::Result<Vec<DbForeignKey>> {
    let mut stmt = conn.prepare(&format!("PRAGMA foreign_key_list({})", quote(SqlDialect::Sqlite, table)))?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>("id")?, r.get::<_, String>("table")?, r.get::<_, String>("from")?, r.get::<_, Option<String>>("to")?)))?;
    let mut out: Vec<(i64, DbForeignKey)> = Vec::new();
    for row in rows {
        let (id, ref_table, from, to) = row?;
        match out.iter_mut().find(|(i, _)| *i == id) {
            Some((_, f)) => {
                f.columns.push(from);
                f.ref_columns.push(to.unwrap_or_default());
            }
            None => out.push((
                id,
                DbForeignKey {
                    name: format!("fk_{table}_{id}"),
                    schema: String::new(),
                    table: table.to_string(),
                    columns: vec![from],
                    ref_schema: String::new(),
                    ref_table,
                    ref_columns: vec![to.unwrap_or_default()],
                },
            )),
        }
    }
    Ok(out.into_iter().map(|(_, f)| f).collect())
}

#[async_trait::async_trait]
impl DbProvider for SqliteProvider {
    fn kind(&self) -> ConnectionType {
        ConnectionType::Sqlite
    }

    async fn load_databases(&self, _cancel: CancellationToken) -> Result<DbMaster> {
        Ok(DbMaster { databases: vec![DbDatabase { name: self.display_name(), ..Default::default() }] })
    }

    async fn load_schema(&self, database: &str, _cancel: CancellationToken) -> Result<DbDatabase> {
        let conn = self.open()?;
        let database = database.to_string();
        tokio::task::spawn_blocking(move || -> Result<DbDatabase> {
            let mut db = DbDatabase { name: database, loaded: true, ..Default::default() };
            let mut stmt = conn
                .prepare("SELECT name, type FROM sqlite_master WHERE type IN ('table','view') ORDER BY type DESC, name ASC")
                .map_err(|e| DataError::Query(e.to_string()))?;
            let entries: Vec<(String, String)> = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .map_err(|e| DataError::Query(e.to_string()))?
                .filter_map(|r| r.ok())
                .filter(|(name, _)| !name.starts_with("sqlite_"))
                .collect();
            drop(stmt);
            for (name, kind) in entries {
                let columns = load_columns(&conn, &name).map_err(|e| DataError::Query(e.to_string()))?;
                if kind != "view" {
                    db.foreign_keys.extend(load_foreign_keys(&conn, &name).unwrap_or_default());
                }
                let t = DbTable { name, schema: String::new(), columns };
                if kind == "view" {
                    db.views.push(t);
                } else {
                    db.tables.push(t);
                }
            }
            Ok(db)
        })
        .await
        .map_err(|e| DataError::Query(e.to_string()))?
    }

    async fn execute(&self, _database: &str, sql: &str, max_rows: u32, cancel: CancellationToken) -> QueryResult {
        let start = std::time::Instant::now();
        let conn = match self.open() {
            Ok(c) => c,
            Err(e) => return QueryResult { error_message: Some(e.to_string()), execution_ms: elapsed_ms(start), ..Default::default() },
        };
        let interrupt = conn.get_interrupt_handle();
        let watcher_cancel = cancel.clone();
        let watcher = tokio::spawn(async move {
            watcher_cancel.cancelled().await;
            interrupt.interrupt();
        });
        let sql = sql.to_string();
        let was_cancelled = cancel.clone();
        let result = tokio::task::spawn_blocking(move || run_batch(&conn, &sql, max_rows)).await;
        watcher.abort();
        let mut result = match result {
            Ok(r) => r,
            Err(e) => QueryResult::failed(e.to_string()),
        };
        if was_cancelled.is_cancelled() {
            result.cancelled = true;
            result.error_message = None;
        }
        result.execution_ms = elapsed_ms(start);
        result
    }

    async fn test(&self, _database: &str, _cancel: CancellationToken) -> Result<()> {
        let conn = self.open()?;
        conn.query_row("SELECT 1", [], |_| Ok(())).map_err(|e| DataError::Connect(e.to_string()))
    }

    async fn close(&self) {}
}

fn run_batch(conn: &rusqlite::Connection, sql: &str, max_rows: u32) -> QueryResult {
    let mut result = QueryResult::default();
    let mut batch = rusqlite::Batch::new(conn, sql);
    let mut affected: i64 = 0;
    loop {
        let stmt = match batch.next() {
            Ok(Some(s)) => s,
            Ok(None) => break,
            Err(e) => {
                set_error(&mut result, e);
                break;
            }
        };
        let mut stmt = stmt;
        let column_count = stmt.column_count();
        if column_count == 0 {
            match stmt.execute([]) {
                Ok(n) => affected += n as i64,
                Err(e) => {
                    set_error(&mut result, e);
                    break;
                }
            }
            continue;
        }
        let mut rs = ResultSet {
            column_names: stmt.column_names().iter().map(|s| s.to_string()).collect(),
            column_types: stmt.columns().iter().map(|c| c.decl_type().map(|t| t.to_string())).collect(),
            ..Default::default()
        };
        let mut rows = match stmt.query([]) {
            Ok(r) => r,
            Err(e) => {
                set_error(&mut result, e);
                break;
            }
        };
        let mut failed = false;
        loop {
            match rows.next() {
                Ok(Some(row)) => {
                    if max_rows > 0 && rs.rows.len() as u32 >= max_rows {
                        rs.truncated = true;
                        break;
                    }
                    let mut cells = Vec::with_capacity(column_count);
                    for i in 0..column_count {
                        cells.push(row.get_ref(i).ok().and_then(format_value));
                        // Fill in the runtime type when the declared type is unknown (expressions).
                        if rs.column_types[i].is_none() {
                            if let Ok(v) = row.get_ref(i) {
                                rs.column_types[i] = match v {
                                    ValueRef::Integer(_) => Some("INTEGER".into()),
                                    ValueRef::Real(_) => Some("REAL".into()),
                                    ValueRef::Text(_) => Some("TEXT".into()),
                                    ValueRef::Blob(_) => Some("BLOB".into()),
                                    ValueRef::Null => None,
                                };
                            }
                        }
                    }
                    rs.rows.push(cells);
                }
                Ok(None) => break,
                Err(e) => {
                    set_error(&mut result, e);
                    failed = true;
                    break;
                }
            }
        }
        result.data.push(rs);
        if failed {
            break;
        }
    }
    if result.error_message.is_none() {
        result.affected_rows = if result.data.is_empty() { affected } else { result.data.iter().map(|d| d.rows.len() as i64).sum() };
    }
    result
}

fn set_error(result: &mut QueryResult, e: rusqlite::Error) {
    result.error_message = Some(e.to_string());
    if let rusqlite::Error::SqliteFailure(ffi, _) = &e {
        result.error_code = Some(ffi.extended_code as i64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(dir: &std::path::Path) -> SqliteProvider {
        SqliteProvider::new(ConnectionConfig {
            kind: ConnectionType::Sqlite,
            file_name: dir.join("t.db").to_string_lossy().into_owned(),
            ..Default::default()
        })
    }

    #[tokio::test]
    async fn schema_and_queries() {
        let dir = tempfile::tempdir().unwrap();
        let p = provider(dir.path());
        let c = CancellationToken::new();
        let r = p
            .execute("", "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT NOT NULL, note TEXT); CREATE VIEW v AS SELECT name FROM people; CREATE TABLE orders (id INTEGER PRIMARY KEY, person_id INTEGER REFERENCES people(id));", 0, c.clone())
            .await;
        assert!(r.error_message.is_none(), "{:?}", r.error_message);

        let r = p.execute("", "INSERT INTO people (name, note) VALUES ('a', NULL), ('b', 'x')", 0, c.clone()).await;
        assert_eq!(r.affected_rows, 2);
        assert!(r.data.is_empty());

        let r = p.execute("", "SELECT id, name, note FROM people ORDER BY id; SELECT COUNT(*) FROM people", 0, c.clone()).await;
        assert_eq!(r.data.len(), 2);
        assert_eq!(r.data[0].column_names, ["id", "name", "note"]);
        assert_eq!(r.data[0].rows[0], vec![Some("1".into()), Some("a".into()), None]);
        assert_eq!(r.data[1].rows[0][0].as_deref(), Some("2"));
        assert_eq!(r.affected_rows, 3);

        let r = p.execute("", "SELECT * FROM people", 1, c.clone()).await;
        assert!(r.data[0].truncated);
        assert_eq!(r.data[0].rows.len(), 1);

        let r = p.execute("", "SELECT * FROM nope", 0, c.clone()).await;
        assert!(r.error_message.unwrap().contains("no such table"));

        let schema = p.load_schema("t.db", c.clone()).await.unwrap();
        assert_eq!(schema.tables.len(), 2);
        assert_eq!(schema.foreign_keys.len(), 1);
        assert_eq!(schema.foreign_keys[0].ref_table, "people");
        assert_eq!(schema.foreign_keys[0].columns, ["person_id"]);
        assert_eq!(schema.foreign_keys[0].ref_columns, ["id"]);
        assert_eq!(schema.views.len(), 1);
        let t = schema.tables.iter().find(|t| t.name == "people").unwrap();
        assert!(t.columns[0].is_primary_key);
        assert!(!t.columns[1].is_nullable);
        assert!(t.columns[2].is_nullable);

        let dbs = p.load_databases(c.clone()).await.unwrap();
        assert_eq!(dbs.databases[0].name, "t.db");
        assert!(p.test("", c).await.is_ok());
    }

    #[tokio::test]
    async fn zero_row_select_keeps_headers() {
        let dir = tempfile::tempdir().unwrap();
        let p = provider(dir.path());
        let c = CancellationToken::new();
        p.execute("", "CREATE TABLE e (a INT)", 0, c.clone()).await;
        let r = p.execute("", "SELECT a FROM e", 0, c).await;
        assert_eq!(r.data.len(), 1);
        assert_eq!(r.data[0].column_names, ["a"]);
        assert!(r.data[0].rows.is_empty());
        assert_eq!(r.affected_rows, 0);
    }
}
