//! SQL Server through tiberius. Authentication: SQL login, Windows (SSPI, Windows builds) or an
//! Azure AD token taken from the Azure CLI (`az login`). Each query uses a fresh connection so a
//! cancelled query is dropped on the floor and the server gets the attention signal.

use crate::cell::{format_bytes, format_f64, format_naive_datetime, format_naive_time, format_offset};
use crate::messages::{self, MessageSink};
use crate::ssh::TunnelManager;
use crate::{elapsed_ms, CancellationToken, DataError, DbProvider, Result};
use dbclient_core::models::*;
use futures::TryStreamExt;
use std::time::Duration;
use tiberius::{AuthMethod, Client, ColumnData, Config, EncryptionLevel, QueryItem, SqlBrowser};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

pub struct SqlServerProvider {
    config: ConnectionConfig,
    tunnel: Option<TunnelManager>,
}

/// (host, port, instance): `host\INSTANCE`, `host,port` and `host:port` forms are all accepted.
fn parse_address(address: &str, port_field: &str) -> (String, Option<u16>, Option<String>) {
    let mut host = address.trim().to_string();
    let mut instance = None;
    let mut port: Option<u16> = port_field.trim().parse().ok().filter(|p: &u16| *p > 0);
    if let Some(idx) = host.find('\\') {
        instance = Some(host[idx + 1..].to_string());
        host = host[..idx].to_string();
    }
    for sep in [',', ':'] {
        if let Some(idx) = host.rfind(sep) {
            if let Ok(p) = host[idx + 1..].trim().parse::<u16>() {
                if port.is_none() {
                    port = Some(p);
                }
                host = host[..idx].to_string();
            }
        }
    }
    (host, port, instance)
}

impl SqlServerProvider {
    pub fn new(config: ConnectionConfig, tunnel: Option<TunnelManager>) -> Self {
        Self { config, tunnel }
    }

    async fn endpoint(&self) -> Result<(String, Option<u16>, Option<String>)> {
        match &self.tunnel {
            Some(t) => Ok(("127.0.0.1".into(), Some(t.local_port().await?), None)),
            None => {
                let (host, port, instance) = parse_address(&self.config.address, &self.config.port);
                if host.is_empty() {
                    return Err(DataError::Config("Server address is empty.".into()));
                }
                Ok((host, port, instance))
            }
        }
    }

    async fn tiberius_config(&self, database: &str) -> Result<Config> {
        let (host, port, instance) = self.endpoint().await?;
        let mut config = Config::new();
        config.host(&host);
        if let Some(p) = port {
            config.port(p);
        } else if instance.is_none() {
            config.port(1433);
        }
        if let Some(i) = instance {
            config.instance_name(i);
        }
        if !database.is_empty() {
            config.database(database);
        }
        config.application_name("dbclient");
        config.command_timeout(Some(Duration::from_secs(self.config.command_timeout.max(1) as u64)));
        if self.config.trust_server_certificate {
            config.trust_cert();
        }
        let azure = self.config.auth_mode == SqlAuthMode::AzureDefault;
        // Encryption is required for Azure SQL; otherwise "Off" still encrypts the login packet.
        config.encryption(if self.config.encrypt || azure { EncryptionLevel::Required } else { EncryptionLevel::Off });
        config.authentication(self.auth().await?);
        Ok(config)
    }

    async fn auth(&self) -> Result<AuthMethod> {
        match self.config.auth_mode {
            SqlAuthMode::SqlLogin => Ok(AuthMethod::sql_server(&self.config.user, &self.config.password)),
            SqlAuthMode::AzureDefault => Ok(AuthMethod::aad_token(azure_token().await?)),
            SqlAuthMode::Windows => {
                #[cfg(windows)]
                {
                    Ok(AuthMethod::Integrated)
                }
                #[cfg(not(windows))]
                {
                    Err(DataError::Config("Windows Authentication is only available on Windows.".into()))
                }
            }
        }
    }

    async fn connect(&self, database: &str, cancel: &CancellationToken) -> Result<Client<Compat<TcpStream>>> {
        let config = self.tiberius_config(database).await?;
        let timeout = Duration::from_secs(self.config.connection_timeout.max(1) as u64);
        let connecting = async {
            let tcp = TcpStream::connect_named(&config).await.map_err(|e| DataError::Connect(describe(&e)))?;
            tcp.set_nodelay(true).ok();
            Client::connect(config, tcp.compat_write()).await.map_err(|e| DataError::Connect(describe(&e)))
        };
        tokio::select! {
            _ = cancel.cancelled() => Err(DataError::Connect("Cancelled.".into())),
            r = tokio::time::timeout(timeout, connecting) => match r {
                Ok(r) => r,
                Err(_) => Err(DataError::Connect(format!("Timed out connecting after {}s.", timeout.as_secs()))),
            },
        }
    }

    /// An ADO.NET-style connection string for SqlPackage, pointing at the tunnel when there is one.
    fn build_connection_string(&self, host: &str, port: Option<u16>, instance: Option<&str>, database: &str) -> String {
        let mut server = host.to_string();
        if let Some(i) = instance {
            server = format!("{server}\\{i}");
        }
        if let Some(p) = port {
            server = format!("{server},{p}");
        }
        let mut parts = vec![
            format!("Data Source={server}"),
            format!("Initial Catalog={database}"),
            format!("Connect Timeout={}", self.config.connection_timeout),
            format!("TrustServerCertificate={}", if self.config.trust_server_certificate { "True" } else { "False" }),
        ];
        match self.config.auth_mode {
            SqlAuthMode::Windows => parts.push("Integrated Security=True".into()),
            SqlAuthMode::AzureDefault => {
                parts.push("Authentication=Active Directory Default".into());
                parts.push("Encrypt=True".into());
            }
            SqlAuthMode::SqlLogin => {
                parts.push(format!("User ID={}", escape_cs(&self.config.user)));
                parts.push(format!("Password={}", escape_cs(&self.config.password)));
            }
        }
        if self.config.auth_mode != SqlAuthMode::AzureDefault {
            parts.push(format!("Encrypt={}", if self.config.encrypt { "True" } else { "False" }));
        }
        parts.join(";")
    }
}

fn escape_cs(v: &str) -> String {
    if v.contains(';') || v.contains('\'') || v.contains('"') || v.starts_with(' ') || v.ends_with(' ') {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v.to_string()
    }
}

/// Access token for Azure SQL: `AZURE_SQL_ACCESS_TOKEN` when set, else the Azure CLI session.
async fn azure_token() -> Result<String> {
    if let Ok(t) = std::env::var("AZURE_SQL_ACCESS_TOKEN") {
        if !t.trim().is_empty() {
            return Ok(t.trim().to_string());
        }
    }
    let args = ["account", "get-access-token", "--resource", "https://database.windows.net/", "--query", "accessToken", "-o", "tsv"];
    let mut cmd = if cfg!(windows) {
        let mut c = tokio::process::Command::new("cmd");
        c.arg("/C").arg("az").args(args);
        c
    } else {
        let mut c = tokio::process::Command::new("az");
        c.args(args);
        c
    };
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let output = tokio::time::timeout(Duration::from_secs(60), cmd.output())
        .await
        .map_err(|_| DataError::Connect("Timed out asking the Azure CLI for a token.".into()))?
        .map_err(|e| DataError::Connect(format!("Couldn't run the Azure CLI (az): {e}. Install it and run `az login`.")))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(DataError::Connect(format!("Azure CLI could not get a token (run `az login`): {err}")));
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if token.is_empty() {
        return Err(DataError::Connect("Azure CLI returned an empty token; run `az login`.".into()));
    }
    Ok(token)
}

fn describe(e: &tiberius::error::Error) -> String {
    match e {
        tiberius::error::Error::Server(s) => s.message().to_string(),
        other => other.to_string(),
    }
}

fn format_cell(data: &ColumnData<'static>) -> Option<String> {
    use tiberius::FromSql;
    match data {
        ColumnData::U8(v) => v.map(|x| x.to_string()),
        ColumnData::I16(v) => v.map(|x| x.to_string()),
        ColumnData::I32(v) => v.map(|x| x.to_string()),
        ColumnData::I64(v) => v.map(|x| x.to_string()),
        ColumnData::F32(v) => v.map(|x| format_f64(x as f64)),
        ColumnData::F64(v) => v.map(format_f64),
        ColumnData::Bit(v) => v.map(|b| if b { "true".into() } else { "false".into() }),
        ColumnData::String(v) => v.as_ref().map(|s| s.to_string()),
        ColumnData::Guid(v) => v.map(|g| g.hyphenated().to_string()),
        ColumnData::Binary(v) => v.as_ref().map(|b| format_bytes(b)),
        ColumnData::Numeric(v) => v.map(|n| n.to_string()),
        ColumnData::Xml(v) => v.as_ref().map(|x| x.to_string()),
        ColumnData::DateTime(_) | ColumnData::SmallDateTime(_) | ColumnData::DateTime2(_) => {
            chrono::NaiveDateTime::from_sql(data).ok().flatten().map(|dt| format_naive_datetime(&dt))
        }
        ColumnData::Date(_) => chrono::NaiveDate::from_sql(data).ok().flatten().map(|d| d.format("%Y-%m-%d").to_string()),
        ColumnData::Time(_) => chrono::NaiveTime::from_sql(data).ok().flatten().map(|t| format_naive_time(&t)),
        ColumnData::DateTimeOffset(_) => chrono::DateTime::<chrono::FixedOffset>::from_sql(data)
            .ok()
            .flatten()
            .map(|dt| format!("{}{}", format_naive_datetime(&dt.naive_local()), format_offset(dt.offset().local_minus_utc() / 60))),
    }
}

fn type_name(t: tiberius::ColumnType) -> String {
    use tiberius::ColumnType as T;
    match t {
        T::Null => "null",
        T::Bit | T::Bitn => "bit",
        T::Int1 => "tinyint",
        T::Int2 => "smallint",
        T::Int4 => "int",
        T::Int8 => "bigint",
        T::Intn => "int",
        T::Datetime4 => "smalldatetime",
        T::Float4 => "real",
        T::Float8 | T::Floatn => "float",
        T::Money | T::Money4 => "money",
        T::Datetime | T::Datetimen => "datetime",
        T::Guid => "uniqueidentifier",
        T::Decimaln | T::Numericn => "decimal",
        T::Daten => "date",
        T::Timen => "time",
        T::Datetime2 => "datetime2",
        T::DatetimeOffsetn => "datetimeoffset",
        T::BigVarBin => "varbinary",
        T::BigVarChar => "varchar",
        T::BigBinary => "binary",
        T::BigChar => "char",
        T::NVarchar => "nvarchar",
        T::NChar => "nchar",
        T::Xml => "xml",
        T::Udt => "udt",
        T::Text => "text",
        T::Image => "image",
        T::NText => "ntext",
        T::SSVariant => "sql_variant",
    }
    .to_string()
}

#[async_trait::async_trait]
impl DbProvider for SqlServerProvider {
    fn kind(&self) -> ConnectionType {
        ConnectionType::SqlServer
    }

    async fn load_databases(&self, cancel: CancellationToken) -> Result<DbMaster> {
        let mut client = self.connect("master", &cancel).await?;
        let rows = client
            .simple_query("SELECT name FROM master.sys.databases WHERE name NOT IN ('master','model','msdb','tempdb') ORDER BY name")
            .await
            .map_err(|e| DataError::Query(describe(&e)))?
            .into_first_result()
            .await
            .map_err(|e| DataError::Query(describe(&e)))?;
        let _ = client.close().await;
        Ok(DbMaster {
            databases: rows
                .iter()
                .filter_map(|r| r.get::<&str, _>(0).map(|s| DbDatabase { name: s.to_string(), ..Default::default() }))
                .collect(),
        })
    }

    async fn load_schema(&self, database: &str, cancel: CancellationToken) -> Result<DbDatabase> {
        let mut db = DbDatabase { name: database.to_string(), loaded: true, ..Default::default() };
        let mut client = self.connect(database, &cancel).await?;
        let q = |e: tiberius::error::Error| DataError::Query(describe(&e));

        let columns_sql = "SELECT CASE WHEN b.TABLE_NAME IS NOT NULL THEN 'view' ELSE 'table' END AS OBJECT_TYPE, \
                           a.TABLE_SCHEMA, a.TABLE_NAME, a.COLUMN_NAME, a.DATA_TYPE, a.IS_NULLABLE, a.CHARACTER_MAXIMUM_LENGTH \
                           FROM INFORMATION_SCHEMA.COLUMNS a \
                           LEFT OUTER JOIN INFORMATION_SCHEMA.VIEWS b \
                             ON a.TABLE_CATALOG = b.TABLE_CATALOG AND a.TABLE_SCHEMA = b.TABLE_SCHEMA AND a.TABLE_NAME = b.TABLE_NAME \
                           ORDER BY a.TABLE_SCHEMA, a.TABLE_NAME, a.ORDINAL_POSITION";
        let rows = client.simple_query(columns_sql).await.map_err(q)?.into_first_result().await.map_err(q)?;
        let mut current: Option<(String, String, bool)> = None;
        for r in rows {
            let object_type: &str = r.get(0).unwrap_or("table");
            let schema: &str = r.get(1).unwrap_or("");
            let table: &str = r.get(2).unwrap_or("");
            let column: &str = r.get(3).unwrap_or("");
            let data_type: &str = r.get(4).unwrap_or("");
            let nullable: &str = r.get(5).unwrap_or("NO");
            let max_len: Option<i32> = r.get(6);
            if schema == "sys" {
                continue;
            }
            let key = (schema.to_string(), table.to_string());
            if current.as_ref().map(|(s, t, _)| (s, t) != (&key.0, &key.1)).unwrap_or(true) {
                let is_view = object_type == "view";
                let t = DbTable { name: table.to_string(), schema: schema.to_string(), columns: Vec::new() };
                if is_view {
                    db.views.push(t);
                } else {
                    db.tables.push(t);
                }
                current = Some((key.0, key.1, is_view));
            }
            let col = DbColumn {
                name: column.to_string(),
                data_type: match max_len {
                    Some(n) if n > 0 => format!("{data_type}({n})"),
                    Some(-1) => format!("{data_type}(max)"),
                    _ => data_type.to_string(),
                },
                is_primary_key: false,
                is_nullable: nullable == "YES",
            };
            if current.as_ref().map(|(_, _, v)| *v).unwrap_or(false) {
                db.views.last_mut().unwrap().columns.push(col);
            } else {
                db.tables.last_mut().unwrap().columns.push(col);
            }
        }

        let pk_sql = "SELECT Col.TABLE_SCHEMA, Col.TABLE_NAME, Col.COLUMN_NAME \
                      FROM INFORMATION_SCHEMA.TABLE_CONSTRAINTS Tab \
                      JOIN INFORMATION_SCHEMA.CONSTRAINT_COLUMN_USAGE Col \
                        ON Col.CONSTRAINT_NAME = Tab.CONSTRAINT_NAME AND Col.TABLE_NAME = Tab.TABLE_NAME \
                      WHERE Tab.CONSTRAINT_TYPE IN ('PRIMARY KEY', 'UNIQUE')";
        let rows = client.simple_query(pk_sql).await.map_err(q)?.into_first_result().await.map_err(q)?;
        for r in rows {
            let schema: &str = r.get(0).unwrap_or("");
            let table: &str = r.get(1).unwrap_or("");
            let column: &str = r.get(2).unwrap_or("");
            if let Some(t) = db.tables.iter_mut().find(|t| t.name.eq_ignore_ascii_case(table) && (t.schema.eq_ignore_ascii_case(schema) || schema.is_empty())) {
                if let Some(c) = t.columns.iter_mut().find(|c| c.name.eq_ignore_ascii_case(column)) {
                    c.is_primary_key = true;
                }
            }
        }

        let sp_sql = "SELECT SPECIFIC_SCHEMA, SPECIFIC_NAME, ROUTINE_TYPE, ISNULL(DATA_TYPE, '') FROM INFORMATION_SCHEMA.ROUTINES \
                      WHERE LEFT(ROUTINE_NAME, 3) NOT IN ('sp_', 'xp_', 'ms_') \
                      ORDER BY SPECIFIC_NAME";
        let rows = client.simple_query(sp_sql).await.map_err(q)?.into_first_result().await.map_err(q)?;
        for r in rows {
            let proc = DbProc {
                schema: r.get::<&str, _>(0).unwrap_or("").to_string(),
                name: r.get::<&str, _>(1).unwrap_or("").to_string(),
                parameters: Vec::new(),
                returns: r.get::<&str, _>(3).unwrap_or("").to_string(),
            };
            if r.get::<&str, _>(2).unwrap_or("") == "FUNCTION" {
                db.functions.push(proc);
            } else {
                db.stored_procedures.push(proc);
            }
        }
        // Parameters (ORDINAL_POSITION 0 is a scalar function's return value).
        let param_sql = "SELECT SPECIFIC_SCHEMA, SPECIFIC_NAME, PARAMETER_NAME, DATA_TYPE, CHARACTER_MAXIMUM_LENGTH, PARAMETER_MODE, ORDINAL_POSITION \
                         FROM INFORMATION_SCHEMA.PARAMETERS ORDER BY SPECIFIC_SCHEMA, SPECIFIC_NAME, ORDINAL_POSITION";
        let param_rows = match client.simple_query(param_sql).await {
            Ok(s) => s.into_first_result().await.unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        {
            for r in param_rows {
                let schema: &str = r.get(0).unwrap_or("");
                let name: &str = r.get(1).unwrap_or("");
                let pname: &str = r.get(2).unwrap_or("");
                let dtype: &str = r.get(3).unwrap_or("");
                let max_len: Option<i32> = r.get(4);
                let mode: &str = r.get(5).unwrap_or("IN");
                let ordinal: i32 = r.get(6).unwrap_or(1);
                let data_type = match max_len {
                    Some(n) if n > 0 => format!("{dtype}({n})"),
                    Some(-1) => format!("{dtype}(max)"),
                    _ => dtype.to_string(),
                };
                if let Some(p) = db.stored_procedures.iter_mut().chain(db.functions.iter_mut()).find(|p| p.name.eq_ignore_ascii_case(name) && p.schema.eq_ignore_ascii_case(schema)) {
                    if ordinal == 0 || pname.is_empty() {
                        if p.returns.is_empty() {
                            p.returns = data_type;
                        }
                    } else {
                        p.parameters.push(DbParameter { name: pname.to_string(), data_type, is_output: mode.contains("OUT") });
                    }
                }
            }
        }
        // Foreign keys.
        let fk_sql = "SELECT fk.name, sch.name, tp.name, cp.name, rsch.name, tr.name, cr.name \
                      FROM sys.foreign_keys fk \
                      JOIN sys.foreign_key_columns fkc ON fkc.constraint_object_id = fk.object_id \
                      JOIN sys.tables tp ON tp.object_id = fk.parent_object_id \
                      JOIN sys.schemas sch ON sch.schema_id = tp.schema_id \
                      JOIN sys.columns cp ON cp.object_id = tp.object_id AND cp.column_id = fkc.parent_column_id \
                      JOIN sys.tables tr ON tr.object_id = fk.referenced_object_id \
                      JOIN sys.schemas rsch ON rsch.schema_id = tr.schema_id \
                      JOIN sys.columns cr ON cr.object_id = tr.object_id AND cr.column_id = fkc.referenced_column_id \
                      ORDER BY fk.name, fkc.constraint_column_id";
        if let Ok(stream) = client.simple_query(fk_sql).await {
            if let Ok(rows) = stream.into_first_result().await {
                for r in rows {
                    let name: &str = r.get(0).unwrap_or("");
                    let schema: &str = r.get(1).unwrap_or("");
                    let table: &str = r.get(2).unwrap_or("");
                    let column: &str = r.get(3).unwrap_or("");
                    let ref_schema: &str = r.get(4).unwrap_or("");
                    let ref_table: &str = r.get(5).unwrap_or("");
                    let ref_column: &str = r.get(6).unwrap_or("");
                    match db.foreign_keys.iter_mut().find(|f| f.name == name && f.schema == schema && f.table == table) {
                        Some(f) => {
                            f.columns.push(column.to_string());
                            f.ref_columns.push(ref_column.to_string());
                        }
                        None => db.foreign_keys.push(DbForeignKey {
                            name: name.to_string(),
                            schema: schema.to_string(),
                            table: table.to_string(),
                            columns: vec![column.to_string()],
                            ref_schema: ref_schema.to_string(),
                            ref_table: ref_table.to_string(),
                            ref_columns: vec![ref_column.to_string()],
                        }),
                    }
                }
            }
        }
        let _ = client.close().await;
        Ok(db)
    }

    async fn execute(&self, database: &str, sql: &str, max_rows: u32, cancel: CancellationToken) -> QueryResult {
        messages::install();
        let start = std::time::Instant::now();
        let mut result = QueryResult::default();
        let sink: MessageSink = Default::default();
        let work = messages::with_sink(sink.clone(), run_batch(self, database, sql, max_rows, &cancel, &mut result));
        tokio::select! {
            _ = cancel.cancelled() => {
                result.cancelled = true;
                result.error_message = None;
                result.data.clear();
            }
            _ = work => {}
        }
        result.messages = sink.lock().map(|m| m.clone()).unwrap_or_default();
        result.execution_ms = elapsed_ms(start);
        result
    }

    async fn test(&self, database: &str, cancel: CancellationToken) -> Result<()> {
        let mut client = self.connect(database, &cancel).await?;
        client.simple_query("SELECT 1").await.map_err(|e| DataError::Connect(describe(&e)))?.into_row().await.map_err(|e| DataError::Connect(describe(&e)))?;
        let _ = client.close().await;
        Ok(())
    }

    async fn external_connection_string(&self, database: &str) -> Result<String> {
        let (host, port, instance) = self.endpoint().await?;
        Ok(self.build_connection_string(&host, port, instance.as_deref(), database))
    }

    async fn close(&self) {
        if let Some(t) = &self.tunnel {
            t.close().await;
        }
    }
}

async fn run_batch(p: &SqlServerProvider, database: &str, sql: &str, max_rows: u32, cancel: &CancellationToken, result: &mut QueryResult) {
    let mut client = match p.connect(database, cancel).await {
        Ok(c) => c,
        Err(e) => {
            result.error_message = Some(e.to_string());
            return;
        }
    };
    let mut truncated = false;
    {
        let mut stream = match client.simple_query(sql).await {
            Ok(s) => s,
            Err(e) => {
                set_error(result, &e);
                return;
            }
        };
        loop {
            match stream.try_next().await {
                Ok(Some(QueryItem::Metadata(meta))) => {
                    result.data.push(ResultSet {
                        column_names: meta.columns().iter().map(|c| c.name().to_string()).collect(),
                        column_types: meta.columns().iter().map(|c| Some(type_name(c.column_type()))).collect(),
                        ..Default::default()
                    });
                }
                Ok(Some(QueryItem::Row(row))) => {
                    let Some(rs) = result.data.last_mut() else { continue };
                    if max_rows > 0 && rs.rows.len() as u32 >= max_rows {
                        // Stop reading: the connection is dropped below, which cancels the batch.
                        rs.truncated = true;
                        truncated = true;
                        break;
                    }
                    rs.rows.push(row.cells().map(|(_, d)| format_cell(d)).collect());
                }
                Ok(None) => break,
                Err(e) => {
                    set_error(result, &e);
                    break;
                }
            }
        }
    }
    if result.error_message.is_none() {
        if result.data.is_empty() {
            // tiberius hides the DONE tokens' counts; @@ROWCOUNT still holds the last statement's.
            result.affected_rows = match client.simple_query("SELECT @@ROWCOUNT").await {
                Ok(s) => s.into_row().await.ok().flatten().and_then(|r| r.get::<i32, _>(0)).unwrap_or(0) as i64,
                Err(_) => 0,
            };
        } else {
            result.affected_rows = result.data.iter().map(|d| d.rows.len() as i64).sum();
        }
    }
    if !truncated {
        let _ = client.close().await;
    }
}

fn set_error(result: &mut QueryResult, e: &tiberius::error::Error) {
    result.error_message = Some(describe(e));
    if let tiberius::error::Error::Server(s) = e {
        result.error_code = Some(s.code() as i64);
        result.error_line = Some(s.line() as i64);
        result.sql_state = Some(s.state().to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_forms() {
        assert_eq!(parse_address("host", "1433"), ("host".into(), Some(1433), None));
        assert_eq!(parse_address("host,1500", ""), ("host".into(), Some(1500), None));
        assert_eq!(parse_address("host:1500", ""), ("host".into(), Some(1500), None));
        assert_eq!(parse_address("host\\SQLEXPRESS", ""), ("host".into(), None, Some("SQLEXPRESS".into())));
        assert_eq!(parse_address("host", ""), ("host".into(), None, None));
    }

    #[test]
    fn connection_string_forms() {
        let p = SqlServerProvider::new(
            ConnectionConfig { user: "sa".into(), password: "p;w".into(), encrypt: false, ..Default::default() },
            None,
        );
        let cs = p.build_connection_string("127.0.0.1", Some(5555), None, "db");
        assert!(cs.contains("Data Source=127.0.0.1,5555"));
        assert!(cs.contains("Initial Catalog=db"));
        assert!(cs.contains("Password=\"p;w\""));
        assert!(cs.contains("Encrypt=False"));

        let p = SqlServerProvider::new(ConnectionConfig { auth_mode: SqlAuthMode::AzureDefault, ..Default::default() }, None);
        let cs = p.build_connection_string("s.database.windows.net", None, None, "db");
        assert!(cs.contains("Authentication=Active Directory Default"));
        assert!(!cs.contains("Password="));
    }
}
