//! Database drivers behind one trait: SQL Server (tiberius), MySQL / MariaDB (mysql_async) and
//! SQLite (rusqlite), each optionally reached through an SSH tunnel (russh).

pub mod messages;
pub mod mysql;
pub mod sqlite;
pub mod sqlserver;
pub mod ssh;
mod cell;

use dbclient_core::models::{ConnectionConfig, ConnectionType, DbDatabase, DbMaster, QueryResult};
use std::sync::Arc;
use thiserror::Error;
pub use tokio_util::sync::CancellationToken;

#[derive(Debug, Error)]
pub enum DataError {
    #[error("{0}")]
    Connect(String),
    #[error("{0}")]
    Ssh(String),
    #[error("{0}")]
    Query(String),
    #[error("{0}")]
    Config(String),
}

pub type Result<T> = std::result::Result<T, DataError>;

/// Details of a server host key that isn't in any known-hosts file yet.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyInfo {
    pub host: String,
    pub port: u16,
    pub key_type: String,
    pub fingerprint_sha256: String,
    pub fingerprint_md5: String,
}

/// Asked whether to trust an unknown SSH host key. Returning `true` pins it to
/// `~/.dbclient/known_hosts`.
pub trait HostKeyPrompt: Send + Sync {
    fn confirm(&self, info: HostKeyInfo) -> futures::future::BoxFuture<'static, bool>;
}

/// One database connection as the app sees it. Each query opens its own network connection, so
/// cancelling is dropping the in-flight future; the SSH tunnel (if any) is kept across queries.
#[async_trait::async_trait]
pub trait DbProvider: Send + Sync {
    fn kind(&self) -> ConnectionType;
    /// Server-level list of user databases.
    async fn load_databases(&self, cancel: CancellationToken) -> Result<DbMaster>;
    /// Tables, views, columns, keys and procedures of one database.
    async fn load_schema(&self, database: &str, cancel: CancellationToken) -> Result<DbDatabase>;
    /// Runs a batch; every outcome (rows, affected count, error, cancellation) is in the result.
    async fn execute(&self, database: &str, sql: &str, max_rows: u32, cancel: CancellationToken) -> QueryResult;
    /// Opens and closes a connection, to prove the settings work.
    async fn test(&self, database: &str, cancel: CancellationToken) -> Result<()>;
    /// Connection string for an external tool (SqlPackage), with the tunnel up. SQL Server only.
    async fn external_connection_string(&self, _database: &str) -> Result<String> {
        Err(DataError::Config("Only SQL Server connections have an external connection string.".into()))
    }
    /// Tear down the tunnel and any pooled resources.
    async fn close(&self);
}

/// Builds the driver for a saved connection. `prompt` handles unknown SSH host keys.
pub fn create_provider(config: &ConnectionConfig, prompt: Arc<dyn HostKeyPrompt>, known_hosts: std::path::PathBuf) -> Result<Arc<dyn DbProvider>> {
    let tunnel = if config.use_ssh && config.kind != ConnectionType::Sqlite {
        Some(ssh::TunnelManager::new(ssh::TunnelSettings::from_config(config)?, prompt, known_hosts))
    } else {
        None
    };
    Ok(match config.kind {
        ConnectionType::SqlServer => Arc::new(sqlserver::SqlServerProvider::new(config.clone(), tunnel)),
        ConnectionType::MySql => Arc::new(mysql::MySqlProvider::new(config.clone(), tunnel)),
        ConnectionType::Sqlite => Arc::new(sqlite::SqliteProvider::new(config.clone())),
    })
}

/// Default database to open for a test / first connection.
pub fn default_database(config: &ConnectionConfig) -> String {
    match config.kind {
        ConnectionType::Sqlite => config.file_name.clone(),
        ConnectionType::MySql => {
            if config.database.is_empty() {
                "information_schema".into()
            } else {
                config.database.clone()
            }
        }
        ConnectionType::SqlServer => {
            if config.database.is_empty() {
                "master".into()
            } else {
                config.database.clone()
            }
        }
    }
}

pub(crate) fn elapsed_ms(start: std::time::Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}
