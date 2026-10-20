//! MySQL / MariaDB through mysql_async (rustls + ring). Schema comes from INFORMATION_SCHEMA and
//! `SHOW KEYS`; server warnings after a query are read with `SHOW WARNINGS`.

use crate::cell::{format_bytes, format_f64, fraction};
use crate::ssh::TunnelManager;
use crate::{elapsed_ms, CancellationToken, DataError, DbProvider, Result};
use dbclient_core::models::*;
use dbclient_core::sql_identifier::{quote, SqlDialect};
use mysql_async::consts::{ColumnFlags, ColumnType};
use mysql_async::prelude::*;
use mysql_async::{Conn, Opts, OptsBuilder, Row, Value};
use std::time::Duration;

pub struct MySqlProvider {
    config: ConnectionConfig,
    tunnel: Option<TunnelManager>,
}

impl MySqlProvider {
    pub fn new(config: ConnectionConfig, tunnel: Option<TunnelManager>) -> Self {
        Self { config, tunnel }
    }

    async fn opts(&self, database: &str) -> Result<Opts> {
        let (host, port) = match &self.tunnel {
            Some(t) => ("127.0.0.1".to_string(), t.local_port().await?),
            None => {
                let port = self.config.port.trim().parse::<u16>().ok().filter(|p| *p > 0).unwrap_or(3306);
                (self.config.address.trim().to_string(), port)
            }
        };
        if host.is_empty() {
            return Err(DataError::Config("Server address is empty.".into()));
        }
        let builder = OptsBuilder::default()
            .ip_or_hostname(host)
            .tcp_port(port)
            .user(Some(self.config.user.clone()))
            .pass(Some(self.config.password.clone()))
            .db_name((!database.is_empty()).then(|| database.to_string()))
            .prefer_socket(false)
            .init(vec!["SET NAMES utf8mb4".to_string()])
            .setup(vec!["SET SESSION sql_mode = CONCAT(@@sql_mode, ',NO_ZERO_DATE')".to_string()]);
        Ok(Opts::from(builder))
    }

    async fn connect(&self, database: &str, cancel: &CancellationToken) -> Result<Conn> {
        let opts = self.opts(database).await?;
        let timeout = Duration::from_secs(self.config.connection_timeout.max(1) as u64);
        let connecting = Conn::new(opts);
        tokio::select! {
            _ = cancel.cancelled() => Err(DataError::Connect("Cancelled.".into())),
            r = tokio::time::timeout(timeout, connecting) => match r {
                Ok(Ok(c)) => Ok(c),
                Ok(Err(e)) => Err(DataError::Connect(describe(&e))),
                Err(_) => Err(DataError::Connect(format!("Timed out connecting after {}s.", timeout.as_secs()))),
            },
        }
    }

    fn command_timeout(&self) -> Duration {
        Duration::from_secs(self.config.command_timeout.max(1) as u64)
    }
}

fn describe(e: &mysql_async::Error) -> String {
    match e {
        mysql_async::Error::Server(s) => s.message.clone(),
        other => other.to_string(),
    }
}

fn type_name(col: &mysql_async::Column) -> String {
    let unsigned = col.flags().contains(ColumnFlags::UNSIGNED_FLAG);
    let binary = col.flags().contains(ColumnFlags::BINARY_FLAG);
    let base = match col.column_type() {
        ColumnType::MYSQL_TYPE_DECIMAL | ColumnType::MYSQL_TYPE_NEWDECIMAL => "decimal",
        ColumnType::MYSQL_TYPE_TINY => "tinyint",
        ColumnType::MYSQL_TYPE_SHORT => "smallint",
        ColumnType::MYSQL_TYPE_LONG => "int",
        ColumnType::MYSQL_TYPE_FLOAT => "float",
        ColumnType::MYSQL_TYPE_DOUBLE => "double",
        ColumnType::MYSQL_TYPE_NULL => "null",
        ColumnType::MYSQL_TYPE_TIMESTAMP | ColumnType::MYSQL_TYPE_TIMESTAMP2 => "timestamp",
        ColumnType::MYSQL_TYPE_LONGLONG => "bigint",
        ColumnType::MYSQL_TYPE_INT24 => "mediumint",
        ColumnType::MYSQL_TYPE_DATE | ColumnType::MYSQL_TYPE_NEWDATE => "date",
        ColumnType::MYSQL_TYPE_TIME | ColumnType::MYSQL_TYPE_TIME2 => "time",
        ColumnType::MYSQL_TYPE_DATETIME | ColumnType::MYSQL_TYPE_DATETIME2 => "datetime",
        ColumnType::MYSQL_TYPE_YEAR => "year",
        ColumnType::MYSQL_TYPE_VARCHAR | ColumnType::MYSQL_TYPE_VAR_STRING => {
            if binary {
                "varbinary"
            } else {
                "varchar"
            }
        }
        ColumnType::MYSQL_TYPE_BIT => "bit",
        ColumnType::MYSQL_TYPE_JSON => "json",
        ColumnType::MYSQL_TYPE_ENUM => "enum",
        ColumnType::MYSQL_TYPE_SET => "set",
        ColumnType::MYSQL_TYPE_TINY_BLOB => {
            if binary {
                "tinyblob"
            } else {
                "tinytext"
            }
        }
        ColumnType::MYSQL_TYPE_MEDIUM_BLOB => {
            if binary {
                "mediumblob"
            } else {
                "mediumtext"
            }
        }
        ColumnType::MYSQL_TYPE_LONG_BLOB => {
            if binary {
                "longblob"
            } else {
                "longtext"
            }
        }
        ColumnType::MYSQL_TYPE_BLOB => {
            if binary {
                "blob"
            } else {
                "text"
            }
        }
        ColumnType::MYSQL_TYPE_STRING => {
            if binary {
                "binary"
            } else {
                "char"
            }
        }
        ColumnType::MYSQL_TYPE_GEOMETRY => "geometry",
        _ => "unknown",
    };
    if unsigned && matches!(base, "tinyint" | "smallint" | "int" | "mediumint" | "bigint" | "decimal" | "float" | "double") {
        format!("{base} unsigned")
    } else {
        base.to_string()
    }
}

fn is_binary_column(col: &mysql_async::Column) -> bool {
    // Character set 63 is `binary`; text columns in other charsets are strings.
    col.character_set() == 63
        && matches!(
            col.column_type(),
            ColumnType::MYSQL_TYPE_VARCHAR
                | ColumnType::MYSQL_TYPE_VAR_STRING
                | ColumnType::MYSQL_TYPE_STRING
                | ColumnType::MYSQL_TYPE_TINY_BLOB
                | ColumnType::MYSQL_TYPE_MEDIUM_BLOB
                | ColumnType::MYSQL_TYPE_LONG_BLOB
                | ColumnType::MYSQL_TYPE_BLOB
                | ColumnType::MYSQL_TYPE_GEOMETRY
        )
}

fn format_value(v: &Value, col: &mysql_async::Column) -> Option<String> {
    match v {
        Value::NULL => None,
        Value::Bytes(b) => {
            if col.column_type() == ColumnType::MYSQL_TYPE_BIT {
                // BIT(n): show as an integer.
                let mut n: u64 = 0;
                for byte in b.iter().take(8) {
                    n = (n << 8) | *byte as u64;
                }
                Some(n.to_string())
            } else if is_binary_column(col) {
                Some(format_bytes(b))
            } else {
                Some(String::from_utf8_lossy(b).into_owned())
            }
        }
        Value::Int(i) => Some(i.to_string()),
        Value::UInt(u) => Some(u.to_string()),
        Value::Float(f) => Some(format_f64(*f as f64)),
        Value::Double(d) => Some(format_f64(*d)),
        Value::Date(y, m, d, h, i, s, us) => {
            let date_only = matches!(col.column_type(), ColumnType::MYSQL_TYPE_DATE | ColumnType::MYSQL_TYPE_NEWDATE);
            if date_only {
                Some(format!("{y:04}-{m:02}-{d:02}"))
            } else {
                Some(format!("{y:04}-{m:02}-{d:02} {h:02}:{i:02}:{s:02}{}", fraction(us * 1000)))
            }
        }
        Value::Time(neg, days, h, m, s, us) => {
            let hours = *days * 24 + *h as u32;
            Some(format!("{}{hours:02}:{m:02}:{s:02}{}", if *neg { "-" } else { "" }, fraction(us * 1000)))
        }
    }
}

fn row_cells(row: &Row) -> Vec<Option<String>> {
    let cols = row.columns_ref();
    (0..cols.len()).map(|i| row.as_ref(i).and_then(|v| format_value(v, &cols[i]))).collect()
}

async fn fetch_warnings(conn: &mut Conn) -> Vec<String> {
    let rows: Vec<Row> = match conn.query("SHOW WARNINGS").await {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    rows.iter()
        .map(|r| {
            let level: String = r.get(0).unwrap_or_default();
            let code: i64 = r.get(1).unwrap_or_default();
            let message: String = r.get(2).unwrap_or_default();
            if level.is_empty() {
                message
            } else {
                format!("{level} {code}: {message}")
            }
        })
        .collect()
}

#[async_trait::async_trait]
impl DbProvider for MySqlProvider {
    fn kind(&self) -> ConnectionType {
        ConnectionType::MySql
    }

    async fn load_databases(&self, cancel: CancellationToken) -> Result<DbMaster> {
        let mut conn = self.connect("information_schema", &cancel).await?;
        let names: Vec<String> = conn.query("SHOW DATABASES").await.map_err(|e| DataError::Query(describe(&e)))?;
        let _ = conn.disconnect().await;
        let excluded = ["mysql", "sys", "performance_schema", "information_schema"];
        Ok(DbMaster {
            databases: names
                .into_iter()
                .filter(|n| !excluded.iter().any(|x| x.eq_ignore_ascii_case(n)))
                .map(|name| DbDatabase { name, ..Default::default() })
                .collect(),
        })
    }

    async fn load_schema(&self, database: &str, cancel: CancellationToken) -> Result<DbDatabase> {
        let mut db = DbDatabase { name: database.to_string(), loaded: true, ..Default::default() };
        let mut conn = self.connect("information_schema", &cancel).await?;
        let sql = "SELECT CASE WHEN b.TABLE_NAME IS NOT NULL THEN 'view' ELSE 'table' END AS OBJECT_TYPE, \
                   a.TABLE_NAME, a.COLUMN_NAME, a.DATA_TYPE, a.IS_NULLABLE, a.CHARACTER_MAXIMUM_LENGTH \
                   FROM INFORMATION_SCHEMA.COLUMNS a \
                   LEFT OUTER JOIN INFORMATION_SCHEMA.VIEWS b \
                     ON a.TABLE_CATALOG = b.TABLE_CATALOG AND a.TABLE_SCHEMA = b.TABLE_SCHEMA AND a.TABLE_NAME = b.TABLE_NAME \
                   WHERE a.TABLE_SCHEMA = ? \
                   ORDER BY a.TABLE_NAME, a.ORDINAL_POSITION";
        let rows: Vec<Row> = conn.exec(sql, (database,)).await.map_err(|e| DataError::Query(describe(&e)))?;
        let mut current: Option<(String, bool)> = None;
        for r in rows {
            let object_type: String = r.get(0).unwrap_or_default();
            let table: String = r.get(1).unwrap_or_default();
            let column: String = r.get(2).unwrap_or_default();
            let data_type: String = r.get(3).unwrap_or_default();
            let nullable: String = r.get(4).unwrap_or_default();
            let max_len: Option<u64> = r.get::<Option<u64>, _>(5).flatten();
            if current.as_ref().map(|(t, _)| t != &table).unwrap_or(true) {
                let is_view = object_type == "view";
                let t = DbTable { name: table.clone(), schema: String::new(), columns: Vec::new() };
                if is_view {
                    db.views.push(t);
                } else {
                    db.tables.push(t);
                }
                current = Some((table.clone(), is_view));
            }
            let col = DbColumn {
                name: column,
                data_type: match max_len {
                    Some(n) => format!("{data_type}({n})"),
                    None => data_type,
                },
                is_primary_key: false,
                is_nullable: nullable == "YES",
            };
            let is_view = current.as_ref().map(|(_, v)| *v).unwrap_or(false);
            if is_view {
                db.views.last_mut().unwrap().columns.push(col);
            } else {
                db.tables.last_mut().unwrap().columns.push(col);
            }
        }
        let _ = conn.disconnect().await;

        if !db.tables.is_empty() {
            let mut conn = self.connect(database, &cancel).await?;
            for table in &mut db.tables {
                if cancel.is_cancelled() {
                    return Err(DataError::Query("Cancelled.".into()));
                }
                let sql = format!("SHOW KEYS FROM {} WHERE Key_name = 'PRIMARY'", quote(SqlDialect::MySql, &table.name));
                match conn.query::<Row, _>(sql).await {
                    Ok(rows) => {
                        for r in rows {
                            let column: Option<String> = r.get("Column_name");
                            if let Some(c) = column {
                                if let Some(col) = table.columns.iter_mut().find(|x| x.name.eq_ignore_ascii_case(&c)) {
                                    col.is_primary_key = true;
                                }
                            }
                        }
                    }
                    Err(e) => dbclient_core_log_warn(&format!("Primary key loading failed for {}: {}", table.name, describe(&e))),
                }
            }
            let _ = conn.disconnect().await;
        }
        // Foreign keys, routines and their parameters from INFORMATION_SCHEMA.
        let mut conn = self.connect("information_schema", &cancel).await?;
        let fk_sql = "SELECT CONSTRAINT_NAME, TABLE_NAME, COLUMN_NAME, REFERENCED_TABLE_NAME, REFERENCED_COLUMN_NAME \
                      FROM INFORMATION_SCHEMA.KEY_COLUMN_USAGE WHERE TABLE_SCHEMA = ? AND REFERENCED_TABLE_NAME IS NOT NULL \
                      ORDER BY CONSTRAINT_NAME, ORDINAL_POSITION";
        if let Ok(rows) = conn.exec::<Row, _, _>(fk_sql, (database,)).await {
            for r in rows {
                let name: String = r.get(0).unwrap_or_default();
                let table: String = r.get(1).unwrap_or_default();
                let column: String = r.get(2).unwrap_or_default();
                let ref_table: String = r.get(3).unwrap_or_default();
                let ref_column: String = r.get(4).unwrap_or_default();
                match db.foreign_keys.iter_mut().find(|f| f.name == name && f.table == table) {
                    Some(f) => {
                        f.columns.push(column);
                        f.ref_columns.push(ref_column);
                    }
                    None => db.foreign_keys.push(DbForeignKey { name, schema: String::new(), table, columns: vec![column], ref_schema: String::new(), ref_table, ref_columns: vec![ref_column] }),
                }
            }
        }
        let routine_sql = "SELECT ROUTINE_NAME, ROUTINE_TYPE, IFNULL(DTD_IDENTIFIER, '') FROM INFORMATION_SCHEMA.ROUTINES WHERE ROUTINE_SCHEMA = ? ORDER BY ROUTINE_NAME";
        if let Ok(rows) = conn.exec::<Row, _, _>(routine_sql, (database,)).await {
            for r in rows {
                let proc = DbProc { name: r.get(0).unwrap_or_default(), schema: String::new(), parameters: Vec::new(), returns: r.get(2).unwrap_or_default() };
                if r.get::<String, _>(1).unwrap_or_default() == "FUNCTION" {
                    db.functions.push(proc);
                } else {
                    db.stored_procedures.push(proc);
                }
            }
        }
        let param_sql = "SELECT SPECIFIC_NAME, PARAMETER_NAME, IFNULL(DTD_IDENTIFIER, ''), IFNULL(PARAMETER_MODE, 'IN') FROM INFORMATION_SCHEMA.PARAMETERS \
                         WHERE SPECIFIC_SCHEMA = ? AND PARAMETER_NAME IS NOT NULL ORDER BY SPECIFIC_NAME, ORDINAL_POSITION";
        if let Ok(rows) = conn.exec::<Row, _, _>(param_sql, (database,)).await {
            for r in rows {
                let name: String = r.get(0).unwrap_or_default();
                let pname: String = r.get(1).unwrap_or_default();
                let data_type: String = r.get(2).unwrap_or_default();
                let mode: String = r.get(3).unwrap_or_default();
                if let Some(p) = db.stored_procedures.iter_mut().chain(db.functions.iter_mut()).find(|p| p.name.eq_ignore_ascii_case(&name)) {
                    p.parameters.push(DbParameter { name: pname, data_type, is_output: mode.contains("OUT") });
                }
            }
        }
        let _ = conn.disconnect().await;
        Ok(db)
    }

    async fn execute(&self, database: &str, sql: &str, max_rows: u32, cancel: CancellationToken) -> QueryResult {
        let start = std::time::Instant::now();
        let mut result = QueryResult::default();
        let mut conn = match self.connect(database, &cancel).await {
            Ok(c) => c,
            Err(e) => {
                result.error_message = Some(e.to_string());
                result.cancelled = cancel.is_cancelled();
                if result.cancelled {
                    result.error_message = None;
                }
                result.execution_ms = elapsed_ms(start);
                return result;
            }
        };
        let work = run_batch(&mut conn, sql, max_rows, &mut result);
        let timed = tokio::time::timeout(self.command_timeout(), work);
        tokio::select! {
            _ = cancel.cancelled() => {
                result.cancelled = true;
                result.error_message = None;
                result.data.clear();
            }
            r = timed => match r {
                Ok(()) => {}
                Err(_) => result.error_message = Some(format!("The query timed out after {}s.", self.command_timeout().as_secs())),
            },
        }
        if !result.cancelled {
            let _ = conn.disconnect().await;
        }
        result.execution_ms = elapsed_ms(start);
        result
    }

    async fn test(&self, database: &str, cancel: CancellationToken) -> Result<()> {
        let mut conn = self.connect(database, &cancel).await?;
        conn.ping().await.map_err(|e| DataError::Connect(describe(&e)))?;
        let _ = conn.disconnect().await;
        Ok(())
    }

    async fn close(&self) {
        if let Some(t) = &self.tunnel {
            t.close().await;
        }
    }
}

fn dbclient_core_log_warn(msg: &str) {
    // The data crate doesn't depend on the store's logger; surface through tracing instead.
    tracing::warn!(target: "dbclient", "{msg}");
}

async fn run_batch(conn: &mut Conn, sql: &str, max_rows: u32, result: &mut QueryResult) {
    let mut affected: i64 = 0;
    let mut warnings: u16 = 0;
    {
        let mut qr = match conn.query_iter(sql).await {
            Ok(q) => q,
            Err(e) => {
                set_error(result, &e);
                return;
            }
        };
        loop {
            let Some(columns) = qr.columns() else {
                // A statement without a result set (DML / DDL).
                affected += qr.affected_rows() as i64;
                warnings = warnings.max(qr.warnings());
                if qr.is_empty() {
                    break;
                }
                // Move past it to the next statement's result.
                match qr.next().await {
                    Ok(None) => {
                        if qr.is_empty() {
                            break;
                        }
                        continue;
                    }
                    Ok(Some(_)) => continue,
                    Err(e) => {
                        set_error(result, &e);
                        return;
                    }
                }
            };
            let mut rs = ResultSet {
                column_names: columns.iter().map(|c| c.name_str().into_owned()).collect(),
                column_types: columns.iter().map(|c| Some(type_name(c))).collect(),
                ..Default::default()
            };
            loop {
                match qr.next().await {
                    Ok(Some(row)) => {
                        if max_rows > 0 && rs.rows.len() as u32 >= max_rows {
                            rs.truncated = true;
                            // Drain the rest of this set so the next one can be read.
                            continue;
                        }
                        rs.rows.push(row_cells(&row));
                    }
                    Ok(None) => break,
                    Err(e) => {
                        set_error(result, &e);
                        result.data.push(rs);
                        return;
                    }
                }
            }
            warnings = warnings.max(qr.warnings());
            result.data.push(rs);
            if qr.is_empty() {
                break;
            }
        }
    }
    result.affected_rows = if result.data.is_empty() { affected } else { result.data.iter().map(|d| d.rows.len() as i64).sum() };
    if warnings > 0 {
        result.messages = fetch_warnings(conn).await;
    }
}

fn set_error(result: &mut QueryResult, e: &mysql_async::Error) {
    result.error_message = Some(describe(e));
    if let mysql_async::Error::Server(s) = e {
        result.error_code = Some(s.code as i64);
        result.sql_state = Some(s.state.clone());
    }
}
