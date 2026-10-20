//! The workspace engine: saved connections and app state, one live session per open connection
//! (driver, database list, schema cache, IntelliSense), query execution with cancellation,
//! query history, SSH host-key prompts and the SqlPackage runner. Tauri-free; the shell wraps it.

pub mod sqlpackage;

use dbclient_core::intellisense::provider::{IntelliSenseProvider, StaticSchema};
use dbclient_core::intellisense::{CompletionItem, Diagnostic, HoverInfo};
use dbclient_core::models::*;
use dbclient_core::{keywords, scripts};
use dbclient_data::{create_provider, CancellationToken, DbProvider, HostKeyInfo, HostKeyPrompt};
use dbclient_store::{logger, Store};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

/// How the engine reaches the UI.
pub trait EventSink: Send + Sync {
    /// An SSH host isn't known yet; the UI shows the fingerprints and calls
    /// [`Workspace::answer_host_key`] with `request_id`.
    fn host_key_prompt(&self, request_id: &str, info: &HostKeyInfo);
    /// A line of SqlPackage output for `job_id`.
    fn sqlpackage_output(&self, job_id: &str, line: &str);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedConnection {
    pub databases: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub data_dir: String,
    pub log_file: String,
    pub os: String,
    pub sqlpackage_path: Option<String>,
}

pub type Result<T> = std::result::Result<T, String>;

struct Session {
    config: ConnectionConfig,
    provider: Arc<dyn DbProvider>,
    databases: Mutex<Vec<String>>,
    schemas: Mutex<HashMap<String, Arc<DbDatabase>>>,
    intellisense: Mutex<HashMap<String, Arc<IntelliSenseProvider>>>,
    /// Recently accepted completions, newest first, to rank them higher.
    recent: Mutex<Vec<String>>,
}

const RECENT_MAX: usize = 50;

pub struct Workspace {
    store: Arc<Store>,
    sink: Arc<dyn EventSink>,
    state: Mutex<AppState>,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    running: Mutex<HashMap<String, CancellationToken>>,
    host_key_waits: Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>,
    sqlpackage_jobs: Mutex<HashMap<String, CancellationToken>>,
    self_ref: Mutex<Weak<Workspace>>,
    version: String,
}

struct Prompt(Weak<Workspace>);

impl HostKeyPrompt for Prompt {
    fn confirm(&self, info: HostKeyInfo) -> futures::future::BoxFuture<'static, bool> {
        let ws = self.0.upgrade();
        Box::pin(async move {
            let Some(ws) = ws else { return false };
            let (tx, rx) = tokio::sync::oneshot::channel();
            let id = new_id();
            ws.host_key_waits.lock().unwrap().insert(id.clone(), tx);
            ws.sink.host_key_prompt(&id, &info);
            let trusted = match tokio::time::timeout(std::time::Duration::from_secs(300), rx).await {
                Ok(Ok(v)) => v,
                _ => false,
            };
            ws.host_key_waits.lock().unwrap().remove(&id);
            logger::info(format!(
                "SSH host key for {}:{} ({}, {}) {} by user",
                info.host,
                info.port,
                info.key_type,
                info.fingerprint_sha256,
                if trusted { "trusted" } else { "rejected" }
            ));
            trusted
        })
    }
}

pub fn new_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let t = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut x = (t as u64) ^ (n.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    // xorshift so consecutive ids don't look alike
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    format!("{:016x}{:016x}", t as u64, x)
}

impl Workspace {
    pub fn new(store: Arc<Store>, sink: Arc<dyn EventSink>, version: String) -> Arc<Workspace> {
        let state = store.load_state();
        let ws = Arc::new(Workspace {
            store,
            sink,
            state: Mutex::new(state),
            sessions: Mutex::new(HashMap::new()),
            running: Mutex::new(HashMap::new()),
            host_key_waits: Mutex::new(HashMap::new()),
            sqlpackage_jobs: Mutex::new(HashMap::new()),
            self_ref: Mutex::new(Weak::new()),
            version,
        });
        *ws.self_ref.lock().unwrap() = Arc::downgrade(&ws);
        ws
    }

    fn prompt(&self) -> Arc<dyn HostKeyPrompt> {
        Arc::new(Prompt(self.self_ref.lock().unwrap().clone()))
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    // --- app state ------------------------------------------------------------

    /// Saved connections with secrets replaced by the placeholder.
    pub fn app_state(&self) -> AppState {
        self.state.lock().unwrap().redacted()
    }

    /// Replaces the app state. Connections carrying the placeholder keep their saved secrets.
    pub fn save_app_state(&self, mut incoming: AppState) -> Result<AppState> {
        let mut state = self.state.lock().unwrap();
        for c in &mut incoming.saved_connections {
            let saved = state.saved_connections.iter().find(|s| s.id == c.id);
            c.restore_secrets(saved);
        }
        incoming.version = state.version.max(1);
        *state = incoming;
        self.store.save_state(&state)?;
        Ok(state.redacted())
    }

    /// Adds or updates a saved connection (new ids are assigned here) and returns it redacted.
    pub fn upsert_connection(&self, mut config: ConnectionConfig) -> Result<ConnectionConfig> {
        let mut state = self.state.lock().unwrap();
        if config.id.is_empty() {
            config.id = new_id();
        }
        let existing = state.saved_connections.iter().position(|c| c.id == config.id);
        config.restore_secrets(existing.map(|i| &state.saved_connections[i]));
        match existing {
            Some(i) => state.saved_connections[i] = config.clone(),
            None => state.saved_connections.push(config.clone()),
        }
        self.store.save_state(&state)?;
        Ok(config.redacted())
    }

    pub fn delete_connection(&self, id: &str) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        state.saved_connections.retain(|c| c.id != id);
        state.open_connection_ids.retain(|c| c != id);
        self.store.save_state(&state)?;
        self.store.delete_connection_state(id);
        Ok(())
    }

    fn saved_connection(&self, id: &str) -> Result<ConnectionConfig> {
        self.state
            .lock()
            .unwrap()
            .saved_connections
            .iter()
            .find(|c| c.id == id)
            .cloned()
            .ok_or_else(|| "That connection is no longer saved.".to_string())
    }

    fn max_rows(&self) -> u32 {
        self.state.lock().unwrap().max_rows
    }

    // --- per-connection session state ---------------------------------------

    pub fn load_connection_state(&self, connection_id: &str) -> Option<ConnectionTabState> {
        self.store.load_connection_state(connection_id)
    }

    pub fn save_connection_state(&self, state: &ConnectionTabState) -> Result<()> {
        self.store.save_connection_state(state)
    }

    // --- sessions ---------------------------------------------------------------

    fn session(&self, id: &str) -> Result<Arc<Session>> {
        self.sessions.lock().unwrap().get(id).cloned().ok_or_else(|| "Not connected.".to_string())
    }

    /// Connects (or reconnects) a saved connection and lists its databases.
    pub async fn open_connection(&self, id: &str) -> Result<OpenedConnection> {
        let config = self.saved_connection(id)?;
        self.close_connection(id).await;
        let provider = create_provider(&config, self.prompt(), self.store.paths.known_hosts_file()).map_err(|e| e.to_string())?;
        let session = Arc::new(Session {
            config: config.clone(),
            provider,
            databases: Mutex::new(Vec::new()),
            schemas: Mutex::new(HashMap::new()),
            intellisense: Mutex::new(HashMap::new()),
            recent: Mutex::new(Vec::new()),
        });
        let cancel = CancellationToken::new();
        let master = session.provider.load_databases(cancel).await.map_err(|e| e.to_string())?;
        let databases: Vec<String> = master.databases.into_iter().map(|d| d.name).collect();
        *session.databases.lock().unwrap() = databases.clone();
        self.sessions.lock().unwrap().insert(id.to_string(), session);
        logger::info(format!("Connected to {} ({} databases)", config.title(), databases.len()));
        Ok(OpenedConnection { databases })
    }

    pub async fn reload_databases(&self, id: &str) -> Result<Vec<String>> {
        let session = self.session(id)?;
        let master = session.provider.load_databases(CancellationToken::new()).await.map_err(|e| e.to_string())?;
        let databases: Vec<String> = master.databases.into_iter().map(|d| d.name).collect();
        *session.databases.lock().unwrap() = databases.clone();
        Ok(databases)
    }

    pub async fn close_connection(&self, id: &str) {
        let removed = self.sessions.lock().unwrap().remove(id);
        if let Some(s) = removed {
            s.provider.close().await;
        }
    }

    pub async fn close_all(&self) {
        let all: Vec<Arc<Session>> = self.sessions.lock().unwrap().drain().map(|(_, s)| s).collect();
        for s in all {
            s.provider.close().await;
        }
        let running: Vec<CancellationToken> = self.running.lock().unwrap().drain().map(|(_, c)| c).collect();
        for c in running {
            c.cancel();
        }
    }

    /// Loads (or returns the cached) schema of one database and prepares IntelliSense for it.
    pub async fn load_schema(&self, id: &str, database: &str, force: bool) -> Result<DbDatabase> {
        let session = self.session(id)?;
        if !force {
            if let Some(s) = session.schemas.lock().unwrap().get(database) {
                return Ok((**s).clone());
            }
        }
        let schema = session.provider.load_schema(database, CancellationToken::new()).await.map_err(|e| e.to_string())?;
        let kind = session.config.kind;
        let intellisense = IntelliSenseProvider::new(Box::new(StaticSchema::from_database(
            &schema,
            keywords::for_dialect(kind),
            keywords::default_schema(kind),
            session.config.dialect(),
        )));
        session.schemas.lock().unwrap().insert(database.to_string(), Arc::new(schema.clone()));
        session.intellisense.lock().unwrap().insert(database.to_string(), Arc::new(intellisense));
        Ok(schema)
    }

    fn intellisense_for(&self, session: &Session, database: &str) -> Arc<IntelliSenseProvider> {
        if let Some(p) = session.intellisense.lock().unwrap().get(database).cloned() {
            return p;
        }
        // Schema not loaded yet: keywords and snippets only.
        let kind = session.config.kind;
        Arc::new(IntelliSenseProvider::new(Box::new(StaticSchema::from_database(
            &DbDatabase::default(),
            keywords::for_dialect(kind),
            keywords::default_schema(kind),
            session.config.dialect(),
        ))))
    }

    pub fn completions(&self, id: &str, database: &str, sql: &str, caret: usize) -> Vec<CompletionItem> {
        let Ok(session) = self.session(id) else { return Vec::new() };
        let recent = session.recent.lock().unwrap().clone();
        self.intellisense_for(&session, database).completions(sql, caret, &recent)
    }

    /// Remember an accepted completion so it ranks first next time.
    pub fn completion_accepted(&self, id: &str, text: &str) {
        let Ok(session) = self.session(id) else { return };
        let mut recent = session.recent.lock().unwrap();
        recent.retain(|r| !r.eq_ignore_ascii_case(text));
        recent.insert(0, text.to_string());
        recent.truncate(RECENT_MAX);
    }

    pub fn hover(&self, id: &str, database: &str, sql: &str, caret: usize) -> Option<HoverInfo> {
        let session = self.session(id).ok()?;
        self.intellisense_for(&session, database).hover(sql, caret)
    }

    pub fn diagnostics(&self, id: &str, database: &str, sql: &str) -> Vec<Diagnostic> {
        let Ok(session) = self.session(id) else { return Vec::new() };
        // Only once the schema is known; otherwise everything would be "unknown".
        let Some(p) = session.intellisense.lock().unwrap().get(database).cloned() else { return Vec::new() };
        p.diagnostics(sql)
    }

    /// Runs a batch on `database`; `job_id` lets the UI cancel it.
    pub async fn execute_query(&self, id: &str, job_id: &str, database: &str, sql: &str) -> Result<QueryResult> {
        let session = self.session(id)?;
        let cancel = CancellationToken::new();
        {
            let mut running = self.running.lock().unwrap();
            if let Some(old) = running.insert(job_id.to_string(), cancel.clone()) {
                old.cancel();
            }
        }
        let result = session.provider.execute(database, sql, self.max_rows(), cancel.clone()).await;
        let mut running = self.running.lock().unwrap();
        if running.get(job_id).is_some_and(|c| c.is_cancelled() == cancel.is_cancelled() && std::ptr::eq(c, &cancel) || true) {
            running.remove(job_id);
        }
        if result.data.iter().any(|d| d.truncated) {
            logger::warn(format!("Result truncated (max rows {}) on {}/{}", self.max_rows(), session.config.title(), database));
        }
        Ok(result)
    }

    pub fn cancel_query(&self, job_id: &str) {
        if let Some(c) = self.running.lock().unwrap().remove(job_id) {
            c.cancel();
        }
    }

    pub async fn test_connection(&self, mut config: ConnectionConfig) -> TestOutcome {
        let saved = self.state.lock().unwrap().saved_connections.iter().find(|c| c.id == config.id).cloned();
        config.restore_secrets(saved.as_ref());
        let provider = match create_provider(&config, self.prompt(), self.store.paths.known_hosts_file()) {
            Ok(p) => p,
            Err(e) => return TestOutcome { ok: false, message: format!("Failed: {e}") },
        };
        let database = dbclient_data::default_database(&config);
        let outcome = match provider.test(&database, CancellationToken::new()).await {
            Ok(()) => TestOutcome { ok: true, message: "Connection successful!".into() },
            Err(e) => TestOutcome { ok: false, message: format!("Failed: {e}") },
        };
        provider.close().await;
        outcome
    }

    pub async fn external_connection_string(&self, id: &str, database: &str) -> Result<String> {
        let session = self.session(id)?;
        session.provider.external_connection_string(database).await.map_err(|e| e.to_string())
    }

    /// Renames a database (SQL Server). Returns the refreshed database list.
    pub async fn rename_database(&self, id: &str, old: &str, new: &str) -> Result<Vec<String>> {
        let session = self.session(id)?;
        let kind = session.config.kind;
        let sql = scripts::rename_database(kind, old, new).ok_or_else(|| format!("Rename is not supported for {} connections.", kind.label()))?;
        let r = session.provider.execute(scripts::admin_database(kind), &sql, 0, CancellationToken::new()).await;
        if let Some(e) = r.error_message {
            return Err(e);
        }
        session.schemas.lock().unwrap().remove(old);
        session.intellisense.lock().unwrap().remove(old);
        self.reload_databases(id).await
    }

    pub async fn drop_database(&self, id: &str, name: &str) -> Result<Vec<String>> {
        let session = self.session(id)?;
        let kind = session.config.kind;
        let sql = scripts::drop_database(kind, name).ok_or_else(|| format!("Delete is not supported for {} connections.", kind.label()))?;
        let r = session.provider.execute(scripts::admin_database(kind), &sql, 0, CancellationToken::new()).await;
        if let Some(e) = r.error_message {
            return Err(e);
        }
        session.schemas.lock().unwrap().remove(name);
        session.intellisense.lock().unwrap().remove(name);
        self.reload_databases(id).await
    }

    // --- history -----------------------------------------------------------------

    pub fn history(&self, connection_id: Option<&str>, filter: Option<&str>) -> Vec<QueryHistoryEntry> {
        self.store.history_for_connection(connection_id, filter)
    }

    pub fn add_history(&self, connection_id: &str, database: &str, query: &str) -> Result<()> {
        let connection = self.saved_connection(connection_id).map(|c| c.title()).unwrap_or_default();
        let query: String = query.chars().take(1000).collect();
        self.store.add_history(QueryHistoryEntry {
            query,
            database: database.to_string(),
            connection,
            connection_id: connection_id.to_string(),
            executed_at: chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false),
        })
    }

    // --- SSH host keys -------------------------------------------------------------

    pub fn answer_host_key(&self, request_id: &str, trust: bool) {
        if let Some(tx) = self.host_key_waits.lock().unwrap().remove(request_id) {
            let _ = tx.send(trust);
        }
    }

    // --- SqlPackage --------------------------------------------------------------------

    pub fn sqlpackage_path(&self) -> Option<String> {
        sqlpackage::executable_path().map(|p| p.to_string_lossy().into_owned())
    }

    pub fn sqlpackage_recheck(&self) -> Option<String> {
        sqlpackage::reset_cache();
        self.sqlpackage_path()
    }

    /// Runs SqlPackage; output lines go to the sink under `job_id`. Returns the exit code.
    pub async fn run_sqlpackage(&self, job_id: &str, args: Vec<String>) -> Result<Option<i32>> {
        let cancel = CancellationToken::new();
        self.sqlpackage_jobs.lock().unwrap().insert(job_id.to_string(), cancel.clone());
        let sink = Arc::clone(&self.sink);
        let job = job_id.to_string();
        let r = sqlpackage::run(&args, cancel, move |line| sink.sqlpackage_output(&job, &line)).await;
        self.sqlpackage_jobs.lock().unwrap().remove(job_id);
        r
    }

    pub fn cancel_sqlpackage(&self, job_id: &str) {
        if let Some(c) = self.sqlpackage_jobs.lock().unwrap().remove(job_id) {
            c.cancel();
        }
    }

    // --- misc -----------------------------------------------------------------------------

    pub fn app_info(&self) -> AppInfo {
        AppInfo {
            version: self.version.clone(),
            data_dir: self.store.paths.root.to_string_lossy().into_owned(),
            log_file: self.store.paths.log_file().to_string_lossy().into_owned(),
            os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
            sqlpackage_path: self.sqlpackage_path(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NullSink;
    impl EventSink for NullSink {
        fn host_key_prompt(&self, _: &str, _: &HostKeyInfo) {}
        fn sqlpackage_output(&self, _: &str, _: &str) {}
    }

    fn workspace(dir: &std::path::Path) -> Arc<Workspace> {
        let store = Arc::new(Store::new(dbclient_store::AppPaths::at(dir)));
        Workspace::new(store, Arc::new(NullSink), "0.0.0".into())
    }

    #[tokio::test]
    async fn sqlite_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let ws = workspace(dir.path());
        let saved = ws
            .upsert_connection(ConnectionConfig {
                kind: ConnectionType::Sqlite,
                display_name: "t".into(),
                file_name: dir.path().join("x.db").to_string_lossy().into_owned(),
                ..Default::default()
            })
            .unwrap();
        assert!(!saved.id.is_empty());
        let opened = ws.open_connection(&saved.id).await.unwrap();
        assert_eq!(opened.databases, ["x.db"]);
        let r = ws.execute_query(&saved.id, "job1", "x.db", "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)").await.unwrap();
        assert!(r.error_message.is_none());
        let schema = ws.load_schema(&saved.id, "x.db", false).await.unwrap();
        assert_eq!(schema.tables[0].name, "t");
        let items = ws.completions(&saved.id, "x.db", "SELECT * FROM ", 14);
        assert!(items.iter().any(|i| i.text == "t"));
        ws.add_history(&saved.id, "x.db", "SELECT 1").unwrap();
        assert_eq!(ws.history(Some(&saved.id), None).len(), 1);
        assert_eq!(ws.history(Some(&saved.id), None)[0].connection, "t");
        ws.close_connection(&saved.id).await;
        assert!(ws.execute_query(&saved.id, "j", "x.db", "SELECT 1").await.is_err());
    }

    #[test]
    fn secrets_round_trip_through_redaction() {
        let dir = tempfile::tempdir().unwrap();
        let ws = workspace(dir.path());
        let saved = ws.upsert_connection(ConnectionConfig { password: "pw".into(), ..Default::default() }).unwrap();
        assert_eq!(saved.password, SAVED_SECRET);
        // Editing with the placeholder keeps the password; a new value replaces it.
        let edited = ws.upsert_connection(ConnectionConfig { display_name: "x".into(), ..saved.clone() }).unwrap();
        assert_eq!(edited.password, SAVED_SECRET);
        assert_eq!(ws.saved_connection(&saved.id).unwrap().password, "pw");
        let mut st = ws.app_state();
        st.saved_connections[0].password = "new".into();
        ws.save_app_state(st).unwrap();
        assert_eq!(ws.saved_connection(&saved.id).unwrap().password, "new");
        ws.delete_connection(&saved.id).unwrap();
        assert!(ws.saved_connection(&saved.id).is_err());
    }

    #[test]
    fn ids_are_unique() {
        let a = new_id();
        let b = new_id();
        assert_ne!(a, b);
        assert_eq!(a.len(), 32);
    }
}
