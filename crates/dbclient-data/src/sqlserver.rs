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

/// Any byte stream a TDS session can run over: TCP, or a Windows named pipe.
pub trait Rw: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> Rw for T {}
type SqlClient = Client<Compat<Box<dyn Rw>>>;

/// `localhost`, `127.0.0.1`, `.`, `(local)`, the machine's own name, or empty.
fn is_local_host(host: &str) -> bool {
    let h = host.trim().to_lowercase();
    if h.is_empty() || matches!(h.as_str(), "localhost" | "127.0.0.1" | "::1" | "." | "(local)" | "local") {
        return true;
    }
    std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).map(|n| n.eq_ignore_ascii_case(&h)).unwrap_or(false)
}

/// `(localdb)\Name` → `Some("Name")`.
#[cfg_attr(not(windows), allow(dead_code))]
fn localdb_instance(address: &str) -> Option<String> {
    let a = address.trim();
    if !a.to_lowercase().starts_with("(localdb)") {
        return None;
    }
    let rest = &a["(localdb)".len()..];
    let name = rest.trim_start_matches(['\\', '/']).trim();
    Some(if name.is_empty() { "MSSQLLocalDB".to_string() } else { name.to_string() })
}

pub struct SqlServerProvider {
    config: ConnectionConfig,
    tunnel: Option<TunnelManager>,
    /// The named pipe that last worked, so later queries skip the refused TCP attempt.
    #[cfg_attr(not(windows), allow(dead_code))]
    known_pipe: std::sync::Mutex<Option<String>>,
}

/// How a connection was made and how long each phase took; shown with the query's messages when
/// connecting dominates the run time.
#[derive(Debug, Default, Clone)]
pub struct ConnectReport {
    /// `TCP host:port` or `named pipe \\.\pipe\...`.
    pub transport: String,
    /// Time to open the socket / pipe.
    pub open_ms: u128,
    /// Time for the TDS prelogin, TLS and login handshake.
    pub handshake_ms: u128,
    /// Endpoints tried before this one, with how long each took to fail.
    pub failed: Vec<String>,
    /// Wall time of the whole attempt, failed endpoints included.
    pub total_ms: u128,
}

impl ConnectReport {
    pub fn describe(&self) -> String {
        let mut text = format!("Connected in {} ms over {}: open {} ms, login {} ms", self.total_ms, self.transport, self.open_ms, self.handshake_ms);
        if !self.failed.is_empty() {
            text.push_str(&format!(" (first tried {})", self.failed.join("; ")));
        }
        text
    }
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
        Self { config, tunnel, known_pipe: std::sync::Mutex::new(None) }
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

    async fn connect(&self, database: &str, cancel: &CancellationToken) -> Result<SqlClient> {
        self.connect_reporting(database, cancel).await.map(|(c, _)| c)
    }

    async fn connect_reporting(&self, database: &str, cancel: &CancellationToken) -> Result<(SqlClient, ConnectReport)> {
        let config = self.tiberius_config(database).await?;
        let timeout = Duration::from_secs(self.config.connection_timeout.max(1) as u64);
        let started = std::time::Instant::now();
        let connecting = self.open_stream(config);
        tokio::select! {
            _ = cancel.cancelled() => Err(DataError::Connect("Cancelled.".into())),
            r = tokio::time::timeout(timeout, connecting) => match r {
                Ok(r) => r.map(|(client, mut report)| {
                    report.total_ms = started.elapsed().as_millis();
                    (client, report)
                }),
                Err(_) => Err(DataError::Connect(format!("Timed out connecting after {}s.", timeout.as_secs()))),
            },
        }
    }

    /// TCP first (through the SQL Browser for named instances); for a local server on Windows,
    /// fall back to the instance's named pipe, which is what dev installs usually listen on.
    /// `(localdb)\Name` goes straight to LocalDB's pipe. A pipe that worked is remembered and
    /// tried first next time, so every query doesn't pay for the refused TCP attempt.
    async fn open_stream(&self, config: Config) -> Result<(SqlClient, ConnectReport)> {
        let (host, port, instance) = parse_address(&self.config.address, &self.config.port);
        let through_tunnel = self.tunnel.is_some();
        let _ = through_tunnel;
        let mut failed: Vec<String> = Vec::new();

        #[cfg(windows)]
        if let (false, Some(name)) = (through_tunnel, localdb_instance(&self.config.address)) {
            let started = std::time::Instant::now();
            let pipe = pipes::localdb_pipe(&name).await?;
            let lookup_ms = started.elapsed().as_millis();
            return pipes::connect_pipe(config, &pipe).await.map(|(c, mut r)| { r.open_ms += lookup_ms; (c, r) })
                .map_err(|e| DataError::Connect(format!("LocalDB instance {name} ({pipe}): {e}")));
        }

        // Taken out of the lock before awaiting: a guard held across the connect would block the
        // reset below and make this future !Send.
        #[cfg(windows)]
        let known_pipe = self.known_pipe.lock().ok().and_then(|p| p.clone());
        #[cfg(windows)]
        if let Some(pipe) = known_pipe {
            match pipes::connect_pipe(config.clone(), &pipe).await {
                Ok(ok) => return Ok(ok),
                Err(e) => {
                    // The server may have restarted with TCP enabled, or the pipe renamed: start over.
                    if let Ok(mut p) = self.known_pipe.lock() {
                        *p = None;
                    }
                    failed.push(format!("named pipe {pipe}: {e}"));
                }
            }
        }

        let endpoint = match (&instance, port) {
            (Some(i), _) => format!("{host}\\{i}"),
            (None, p) => format!("{host}:{}", p.unwrap_or(1433)),
        };
        let started = std::time::Instant::now();
        let tcp_result = async {
            let tcp = TcpStream::connect_named(&config).await.map_err(|e| describe(&e))?;
            tcp.set_nodelay(true).ok();
            let open_ms = started.elapsed().as_millis();
            let boxed: Box<dyn Rw> = Box::new(tcp);
            let client = Client::connect(config.clone(), boxed.compat_write()).await.map_err(|e| describe(&e))?;
            let handshake_ms = started.elapsed().as_millis() - open_ms;
            Ok::<_, String>((client, open_ms, handshake_ms))
        }
        .await;
        let tcp_error = match tcp_result {
            Ok((client, open_ms, handshake_ms)) => {
                #[cfg(windows)]
                if let Ok(mut p) = self.known_pipe.lock() {
                    *p = None;
                }
                return Ok((client, ConnectReport { transport: format!("TCP {endpoint}"), open_ms, handshake_ms, failed, total_ms: 0 }));
            }
            Err(e) => e,
        };
        failed.push(format!("TCP {endpoint}: {tcp_error} ({} ms)", started.elapsed().as_millis()));

        #[cfg(windows)]
        if !through_tunnel && is_local_host(&host) {
            let candidates: Vec<String> = match &instance {
                Some(i) => vec![format!(r"\\.\pipe\MSSQL${i}\sql\query")],
                None => vec![r"\\.\pipe\sql\query".to_string(), r"\\.\pipe\MSSQL$SQLEXPRESS\sql\query".to_string()],
            };
            for pipe in candidates {
                let started = std::time::Instant::now();
                match pipes::connect_pipe(config.clone(), &pipe).await {
                    Ok((client, mut report)) => {
                        if let Ok(mut p) = self.known_pipe.lock() {
                            *p = Some(pipe);
                        }
                        report.failed = failed;
                        return Ok((client, report));
                    }
                    Err(e) => failed.push(format!("named pipe {pipe}: {e} ({} ms)", started.elapsed().as_millis())),
                }
            }
            return Err(DataError::Connect(format!(
                "Could not reach the local SQL Server. Tried {}. Check that the instance is running, and either enable TCP/IP in SQL Server Configuration Manager or connect as host\\INSTANCE.",
                failed.join("; ")
            )));
        }
        let _ = is_local_host; // keep the helper used on every platform
        let _ = &failed;
        Err(DataError::Connect(format!("{tcp_error} ({endpoint})")))
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

#[cfg(windows)]
mod pipes {
    use super::{describe, ConnectReport, Rw, SqlClient};
    use crate::{DataError, Result};
    use std::time::{Duration, Instant};
    use tiberius::{Client, Config};
    use tokio::net::windows::named_pipe::ClientOptions;
    use tokio_util::compat::TokioAsyncWriteCompatExt;

    const ERROR_PIPE_BUSY: i32 = 231;

    pub async fn connect_pipe(config: Config, path: &str) -> std::result::Result<(SqlClient, ConnectReport), String> {
        let started = Instant::now();
        // The server's pipe may be momentarily busy while another client connects.
        let pipe = loop {
            match ClientOptions::new().open(path) {
                Ok(p) => break p,
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => tokio::time::sleep(Duration::from_millis(50)).await,
                Err(e) => return Err(e.to_string()),
            }
        };
        let open_ms = started.elapsed().as_millis();
        let boxed: Box<dyn Rw> = Box::new(pipe);
        let client = Client::connect(config, boxed.compat_write()).await.map_err(|e| describe(&e))?;
        let handshake_ms = started.elapsed().as_millis() - open_ms;
        Ok((client, ConnectReport { transport: format!("named pipe {path}"), open_ms, handshake_ms, failed: Vec::new(), total_ms: 0 }))
    }

    /// Start the LocalDB instance if needed and return its pipe (`sqllocaldb info <name>`).
    pub async fn localdb_pipe(name: &str) -> Result<String> {
        let run = |args: &[&str]| {
            let mut c = tokio::process::Command::new("sqllocaldb");
            c.args(args);
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x0800_0000);
            c.output()
        };
        let parse = |out: &std::process::Output| {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .find_map(|l| l.trim().strip_prefix("Instance pipe name:").map(|p| p.trim().trim_start_matches("np:").to_string()))
                .filter(|p| !p.is_empty())
        };
        let info = run(&["info", name]).await.map_err(|e| DataError::Connect(format!("Couldn't run sqllocaldb: {e}. Is SQL Server LocalDB installed?")))?;
        if !info.status.success() {
            return Err(DataError::Connect(format!("LocalDB instance '{name}' not found: {}", String::from_utf8_lossy(&info.stderr).trim())));
        }
        if let Some(p) = parse(&info) {
            return Ok(p);
        }
        let started = run(&["start", name]).await.map_err(|e| DataError::Connect(e.to_string()))?;
        if !started.status.success() {
            return Err(DataError::Connect(format!("Couldn't start LocalDB instance '{name}': {}", String::from_utf8_lossy(&started.stderr).trim())));
        }
        let info = run(&["info", name]).await.map_err(|e| DataError::Connect(e.to_string()))?;
        parse(&info).ok_or_else(|| DataError::Connect(format!("LocalDB instance '{name}' has no pipe name; is it running?")))
    }
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
        let mut connect_notes = Vec::new();
        let work = messages::with_sink(sink.clone(), run_batch(self, database, sql, max_rows, &cancel, &mut result, &mut connect_notes));
        tokio::select! {
            _ = cancel.cancelled() => {
                result.cancelled = true;
                result.error_message = None;
                result.data.clear();
            }
            _ = work => {}
        }
        result.messages = sink.lock().map(|m| m.clone()).unwrap_or_default();
        result.messages.splice(0..0, connect_notes);
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

/// Connects at least this slow get a note in the query's messages saying where the time went.
const SLOW_CONNECT_MS: u128 = 500;

async fn run_batch(p: &SqlServerProvider, database: &str, sql: &str, max_rows: u32, cancel: &CancellationToken, result: &mut QueryResult, connect_notes: &mut Vec<String>) {
    let (mut client, report) = match p.connect_reporting(database, cancel).await {
        Ok(c) => c,
        Err(e) => {
            result.error_message = Some(e.to_string());
            return;
        }
    };
    if report.total_ms >= SLOW_CONNECT_MS {
        connect_notes.push(report.describe());
    }
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
        // The logout round trip isn't part of the query; don't make the result wait for it.
        tokio::spawn(async move {
            let _ = tokio::time::timeout(Duration::from_secs(5), client.close()).await;
        });
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
    fn local_hosts_and_localdb() {
        assert!(is_local_host("localhost"));
        assert!(is_local_host("."));
        assert!(is_local_host("(local)"));
        assert!(!is_local_host("db.example.com"));
        assert_eq!(localdb_instance("(localdb)\\MSSQLLocalDB").as_deref(), Some("MSSQLLocalDB"));
        assert_eq!(localdb_instance("(LocalDB)").as_deref(), Some("MSSQLLocalDB"));
        assert!(localdb_instance("localhost").is_none());
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
