//! DTOs shared between the Rust side and the webview. Everything here is
//! serialized camelCase and mirrored by hand in `src/lib/types.ts`.

use serde::{Deserialize, Serialize};

/// Which database engine a connection talks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionType {
    #[default]
    SqlServer,
    MySql,
    Sqlite,
}

impl ConnectionType {
    pub fn label(self) -> &'static str {
        match self {
            ConnectionType::SqlServer => "SQL Server",
            ConnectionType::MySql => "MySQL",
            ConnectionType::Sqlite => "SQLite",
        }
    }
}

/// SQL Server authentication: SQL login, Azure (token from `az login`), or the
/// logged-in Windows account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SqlAuthMode {
    #[default]
    SqlLogin,
    AzureDefault,
    Windows,
}

/// A saved connection. Secrets (`password`, `ssh_password`, `ssh_key_passphrase`)
/// are kept encrypted on disk and never sent to the webview (see [`ConnectionConfig::redacted`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ConnectionConfig {
    pub id: String,
    pub display_name: String,
    #[serde(rename = "type", deserialize_with = "de::connection_type")]
    pub kind: ConnectionType,
    pub address: String,
    pub user: String,
    pub password: String,
    pub port: String,
    pub database: String,
    #[serde(deserialize_with = "de::auth_mode")]
    pub auth_mode: SqlAuthMode,
    /// SQLite file.
    pub file_name: String,
    pub connection_timeout: u32,
    pub command_timeout: u32,
    #[serde(rename = "useSSH", alias = "useSsh")]
    pub use_ssh: bool,
    pub ssh_host: String,
    pub ssh_user: String,
    pub ssh_password: String,
    pub ssh_remote_port: u16,
    pub ssh_key_file: String,
    pub ssh_key_passphrase: String,
    /// Encrypt the SQL Server connection. Default on.
    pub encrypt: bool,
    /// Trust the server certificate without validation (self-signed dev servers only).
    pub trust_server_certificate: bool,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            display_name: String::new(),
            kind: ConnectionType::SqlServer,
            address: String::new(),
            user: String::new(),
            password: String::new(),
            port: String::new(),
            database: String::new(),
            auth_mode: SqlAuthMode::SqlLogin,
            file_name: String::new(),
            connection_timeout: 15,
            command_timeout: 30,
            use_ssh: false,
            ssh_host: String::new(),
            ssh_user: String::new(),
            ssh_password: String::new(),
            ssh_remote_port: 0,
            ssh_key_file: String::new(),
            ssh_key_passphrase: String::new(),
            encrypt: true,
            trust_server_certificate: false,
        }
    }
}

/// Placeholder the webview gets instead of a saved secret; sending it back means "keep what's saved".
pub const SAVED_SECRET: &str = "\u{0}saved\u{0}";

impl ConnectionConfig {
    /// Display name, falling back to the file name (SQLite) or `Type: address`.
    pub fn title(&self) -> String {
        if !self.display_name.is_empty() {
            return self.display_name.clone();
        }
        match self.kind {
            ConnectionType::Sqlite => std::path::Path::new(&self.file_name)
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default(),
            kind => format!("{}: {}", kind.label(), self.address),
        }
    }

    /// Copy with every set secret replaced by [`SAVED_SECRET`].
    pub fn redacted(&self) -> ConnectionConfig {
        let mut c = self.clone();
        for s in [&mut c.password, &mut c.ssh_password, &mut c.ssh_key_passphrase] {
            if !s.is_empty() {
                *s = SAVED_SECRET.to_string();
            }
        }
        c
    }

    /// Put saved secrets back where the webview left the placeholder.
    pub fn restore_secrets(&mut self, saved: Option<&ConnectionConfig>) {
        let restore = |field: &mut String, from: Option<&String>| {
            if field == SAVED_SECRET {
                *field = from.cloned().unwrap_or_default();
            }
        };
        restore(&mut self.password, saved.map(|s| &s.password));
        restore(&mut self.ssh_password, saved.map(|s| &s.ssh_password));
        restore(&mut self.ssh_key_passphrase, saved.map(|s| &s.ssh_key_passphrase));
    }

    pub fn dialect(&self) -> crate::SqlDialect {
        match self.kind {
            ConnectionType::MySql => crate::SqlDialect::MySql,
            ConnectionType::Sqlite => crate::SqlDialect::Sqlite,
            ConnectionType::SqlServer => crate::SqlDialect::SqlServer,
        }
    }
}

/// Accept both the enum names this app writes and the integers the .NET app wrote.
mod de {
    use super::{ConnectionType, SqlAuthMode};
    use serde::{Deserialize, Deserializer};

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum IntOr<T> {
        Int(i64),
        Named(T),
    }

    pub fn connection_type<'de, D: Deserializer<'de>>(d: D) -> Result<ConnectionType, D::Error> {
        Ok(match IntOr::<ConnectionType>::deserialize(d)? {
            IntOr::Named(t) => t,
            IntOr::Int(1) => ConnectionType::MySql,
            IntOr::Int(2) => ConnectionType::Sqlite,
            IntOr::Int(_) => ConnectionType::SqlServer,
        })
    }

    pub fn auth_mode<'de, D: Deserializer<'de>>(d: D) -> Result<SqlAuthMode, D::Error> {
        Ok(match IntOr::<SqlAuthMode>::deserialize(d)? {
            IntOr::Named(t) => t,
            IntOr::Int(1) => SqlAuthMode::AzureDefault,
            IntOr::Int(2) => SqlAuthMode::Windows,
            IntOr::Int(_) => SqlAuthMode::SqlLogin,
        })
    }
}

// --- persisted app state ------------------------------------------------------

/// `~/.dbclient/state.json`: saved connections, which are open, layout and editor settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppState {
    pub version: u32,
    pub saved_connections: Vec<ConnectionConfig>,
    pub open_connection_ids: Vec<String>,
    pub active_connection_tab_id: Option<String>,
    pub is_connection_panel_open: bool,
    pub is_history_panel_open: bool,
    /// "Dark", "Dracula" or "Light".
    pub theme: String,
    /// Max rows fetched per result set (0 = unlimited).
    pub max_rows: u32,
    pub editor_font_size: f64,
    pub editor_word_wrap: bool,
    pub left_panel_width: Option<f64>,
    /// Editor height as a fraction of editor + results height (0..1).
    pub editor_height_ratio: Option<f64>,
    pub history_panel_width: Option<f64>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            version: 1,
            saved_connections: Vec::new(),
            open_connection_ids: Vec::new(),
            active_connection_tab_id: None,
            is_connection_panel_open: true,
            is_history_panel_open: false,
            theme: "Dark".into(),
            max_rows: 100_000,
            editor_font_size: 14.0,
            editor_word_wrap: false,
            left_panel_width: None,
            editor_height_ratio: None,
            history_panel_width: None,
        }
    }
}

impl AppState {
    pub fn redacted(&self) -> AppState {
        let mut s = self.clone();
        s.saved_connections = s.saved_connections.iter().map(ConnectionConfig::redacted).collect();
        s
    }
}

/// `~/.dbclient/connections/<id>.json`: a connection's query tabs, grouped by database.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ConnectionTabState {
    pub id: String,
    pub connection_id: String,
    pub active_database: String,
    pub query_tabs: Vec<TabState>,
    pub active_query_tab_id: Option<String>,
    pub active_query_tab_by_database: std::collections::HashMap<String, Option<String>>,
}

impl Default for ConnectionTabState {
    fn default() -> Self {
        Self {
            id: String::new(),
            connection_id: String::new(),
            active_database: String::new(),
            query_tabs: Vec::new(),
            active_query_tab_id: None,
            active_query_tab_by_database: Default::default(),
        }
    }
}

impl ConnectionTabState {
    /// Older files: tabs without a database belong to the active one, and the single
    /// `activeQueryTabId` moves into the per-database map.
    pub fn normalize(&mut self) {
        for t in &mut self.query_tabs {
            if t.database.is_empty() {
                t.database = self.active_database.clone();
            }
        }
        if let Some(id) = &self.active_query_tab_id {
            if !self.active_database.is_empty() && !self.active_query_tab_by_database.contains_key(&self.active_database) {
                self.active_query_tab_by_database.insert(self.active_database.clone(), Some(id.clone()));
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct TabState {
    pub id: String,
    pub title: String,
    pub query_text: String,
    pub order: i32,
    pub database: String,
}

/// One executed query. Written camelCase; the PascalCase aliases read the `history.json` the
/// .NET app wrote with default System.Text.Json naming.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct QueryHistoryEntry {
    #[serde(alias = "Query")]
    pub query: String,
    #[serde(alias = "Database")]
    pub database: String,
    #[serde(alias = "Connection")]
    pub connection: String,
    #[serde(alias = "ConnectionId")]
    pub connection_id: String,
    /// ISO-8601 local time.
    #[serde(alias = "ExecutedAt")]
    pub executed_at: String,
}

// --- schema ----------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DbMaster {
    pub databases: Vec<DbDatabase>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct DbDatabase {
    pub name: String,
    pub loaded: bool,
    pub tables: Vec<DbTable>,
    pub views: Vec<DbTable>,
    pub stored_procedures: Vec<DbProc>,
    pub functions: Vec<DbProc>,
    pub foreign_keys: Vec<DbForeignKey>,
}

/// A foreign key: `schema.table(columns)` references `ref_schema.ref_table(ref_columns)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct DbForeignKey {
    pub name: String,
    pub schema: String,
    pub table: String,
    pub columns: Vec<String>,
    pub ref_schema: String,
    pub ref_table: String,
    pub ref_columns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct DbParameter {
    pub name: String,
    pub data_type: String,
    pub is_output: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DbTable {
    pub name: String,
    pub schema: String,
    pub columns: Vec<DbColumn>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct DbProc {
    pub name: String,
    pub schema: String,
    pub parameters: Vec<DbParameter>,
    /// Return type for functions; empty for procedures.
    pub returns: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DbColumn {
    pub name: String,
    pub data_type: String,
    pub is_primary_key: bool,
    pub is_nullable: bool,
}

// --- results ----------------------------------------------------------------------

/// One result set. Cells are display strings; `None` is SQL NULL.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ResultSet {
    pub column_names: Vec<String>,
    pub column_types: Vec<Option<String>>,
    pub rows: Vec<Vec<Option<String>>>,
    /// True when the row cap stopped reading before the set was exhausted.
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct QueryResult {
    pub data: Vec<ResultSet>,
    pub affected_rows: i64,
    pub error_message: Option<String>,
    pub error_code: Option<i64>,
    pub sql_state: Option<String>,
    pub error_line: Option<i64>,
    pub execution_ms: f64,
    /// Server info messages (PRINT, warnings).
    pub messages: Vec<String>,
    pub cancelled: bool,
}

impl QueryResult {
    pub fn is_error(&self) -> bool {
        self.error_message.is_some()
    }

    pub fn failed(message: impl Into<String>) -> Self {
        QueryResult { error_message: Some(message.into()), ..Default::default() }
    }
}

/// What the test-connection button reports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TestOutcome {
    pub ok: bool,
    pub message: String,
}
