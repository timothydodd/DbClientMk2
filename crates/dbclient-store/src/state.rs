//! `state.json` (saved connections, open tabs, layout), `connections/<id>.json` (query tabs per
//! connection) and `history.json`. Writes are atomic; the previous `state.json` is kept as `.bak`
//! and a corrupt file is quarantined before the backup is used.

use crate::logger;
use crate::paths::{write_text_atomic, AppPaths};
use crate::secrets::{is_encrypted, Protector};
use dbclient_core::models::{AppState, ConnectionTabState, QueryHistoryEntry};
use std::path::Path;
use std::sync::Mutex;

pub const CURRENT_VERSION: u32 = 1;
const MAX_HISTORY: usize = 100;

pub struct Store {
    pub paths: AppPaths,
    protector: Protector,
    lock: Mutex<()>,
    /// Set when the loaded file came from a newer app version; back it up before the first save.
    newer_version_backup_pending: Mutex<bool>,
}

impl Store {
    pub fn new(paths: AppPaths) -> Self {
        let protector = Protector::new(paths.clone());
        Self { paths, protector, lock: Mutex::new(()), newer_version_backup_pending: Mutex::new(false) }
    }

    // --- app state -------------------------------------------------------------

    pub fn load_state(&self) -> AppState {
        let file = self.paths.state_file();
        if !file.exists() {
            return AppState::default();
        }
        let (mut state, corrupt) = read_state_file(&file);
        if state.is_none() && corrupt {
            let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
            let quarantined = self.paths.root.join(format!("state.json.corrupt-{stamp}"));
            match std::fs::rename(&file, &quarantined) {
                Ok(()) => logger::warn(format!("State file was unreadable; moved to {}", quarantined.display())),
                Err(e) => logger::error(format!("Failed to quarantine corrupt state file: {e}")),
            }
            let bak = self.backup_file();
            if bak.exists() {
                state = read_state_file(&bak).0;
                if state.is_some() {
                    logger::info("Recovered state from state.json.bak");
                }
            }
        }
        let Some(mut state) = state else { return AppState::default() };

        if state.version > CURRENT_VERSION {
            logger::warn(format!(
                "State file version {} is newer than supported {CURRENT_VERSION}; it will be backed up before any save",
                state.version
            ));
            *self.newer_version_backup_pending.lock().unwrap() = true;
        } else if state.version < CURRENT_VERSION {
            logger::info(format!("Migrating state file from version {} to {CURRENT_VERSION}", state.version));
            state.version = CURRENT_VERSION;
        }
        self.decrypt_passwords(&mut state);
        state
    }

    pub fn save_state(&self, state: &AppState) -> Result<(), String> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        self.paths.ensure_directory().map_err(|e| e.to_string())?;
        let file = self.paths.state_file();

        let mut pending = self.newer_version_backup_pending.lock().unwrap();
        if *pending && file.exists() {
            let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
            let backup = self.paths.root.join(format!("state.json.newer-{stamp}"));
            if std::fs::copy(&file, &backup).is_ok() {
                crate::paths::restrict_file(&backup);
                logger::warn(format!("Backed up newer-version state file to {}", backup.display()));
            }
            *pending = false;
        }
        drop(pending);

        let mut clone = state.clone();
        clone.version = CURRENT_VERSION;
        self.encrypt_passwords(&mut clone)?;
        let json = serde_json::to_string_pretty(&clone).map_err(|e| e.to_string())?;

        if file.exists() {
            let bak = self.backup_file();
            if std::fs::copy(&file, &bak).is_ok() {
                crate::paths::restrict_file(&bak);
            }
        }
        write_text_atomic(&file, &json).map_err(|e| e.to_string())
    }

    fn backup_file(&self) -> std::path::PathBuf {
        self.paths.root.join("state.json.bak")
    }

    fn encrypt_passwords(&self, state: &mut AppState) -> Result<(), String> {
        for c in &mut state.saved_connections {
            for s in [&mut c.password, &mut c.ssh_password, &mut c.ssh_key_passphrase] {
                if !s.is_empty() && !is_encrypted(s) {
                    *s = self.protector.encrypt(s)?;
                }
            }
        }
        Ok(())
    }

    fn decrypt_passwords(&self, state: &mut AppState) {
        for c in &mut state.saved_connections {
            let name = c.title();
            for (label, s) in [("Password", &mut c.password), ("SSH password", &mut c.ssh_password), ("SSH key passphrase", &mut c.ssh_key_passphrase)] {
                match self.protector.try_decrypt(s) {
                    Ok(plain) => *s = plain,
                    Err(e) => {
                        logger::warn(format!("{label} for connection '{name}' could not be decrypted ({e}); re-enter it in the connection dialog"));
                        *s = String::new();
                    }
                }
            }
        }
    }

    // --- per-connection session state ---------------------------------------

    pub fn load_connection_state(&self, connection_id: &str) -> Option<ConnectionTabState> {
        let file = self.paths.connections_dir().join(format!("{connection_id}.json"));
        let json = std::fs::read_to_string(&file).ok()?;
        match serde_json::from_str::<ConnectionTabState>(&json) {
            Ok(mut s) => {
                s.normalize();
                Some(s)
            }
            Err(e) => {
                logger::error(format!("Failed to load connection state {connection_id}: {e}"));
                None
            }
        }
    }

    pub fn save_connection_state(&self, state: &ConnectionTabState) -> Result<(), String> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let file = self.paths.connections_dir().join(format!("{}.json", state.connection_id));
        let json = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
        write_text_atomic(&file, &json).map_err(|e| e.to_string())
    }

    pub fn delete_connection_state(&self, connection_id: &str) {
        let file = self.paths.connections_dir().join(format!("{connection_id}.json"));
        if file.exists() {
            if let Err(e) = std::fs::remove_file(&file) {
                logger::error(format!("Failed to delete connection state {connection_id}: {e}"));
            }
        }
    }

    // --- query history --------------------------------------------------------

    pub fn load_history(&self) -> Vec<QueryHistoryEntry> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        self.read_history()
    }

    fn read_history(&self) -> Vec<QueryHistoryEntry> {
        let file = self.paths.history_file();
        let Ok(json) = std::fs::read_to_string(&file) else { return Vec::new() };
        match serde_json::from_str(&json) {
            Ok(v) => v,
            Err(e) => {
                logger::error(format!("Failed to load query history: {e}"));
                Vec::new()
            }
        }
    }

    /// Read-modify-write under the store lock so concurrent adds never drop entries.
    pub fn add_history(&self, entry: QueryHistoryEntry) -> Result<(), String> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut history = self.read_history();
        history.insert(0, entry);
        history.truncate(MAX_HISTORY);
        let json = serde_json::to_string_pretty(&history).map_err(|e| e.to_string())?;
        write_text_atomic(&self.paths.history_file(), &json).map_err(|e| e.to_string())
    }

    pub fn history_for_connection(&self, connection_id: Option<&str>, filter: Option<&str>) -> Vec<QueryHistoryEntry> {
        let all = self.load_history();
        let filter = filter.map(|f| f.trim().to_lowercase()).filter(|f| !f.is_empty());
        all.into_iter()
            .filter(|e| connection_id.is_none_or(|id| e.connection_id == id))
            .filter(|e| {
                filter.as_ref().is_none_or(|f| {
                    e.query.to_lowercase().contains(f) || e.database.to_lowercase().contains(f) || e.connection.to_lowercase().contains(f)
                })
            })
            .collect()
    }
}

/// `(state, corrupt)`: `corrupt` is true when the file exists but can't be parsed.
fn read_state_file(path: &Path) -> (Option<AppState>, bool) {
    match std::fs::read_to_string(path) {
        Ok(json) => match serde_json::from_str::<Option<AppState>>(&json) {
            Ok(Some(s)) => (Some(s), false),
            Ok(None) => {
                logger::warn(format!("{} deserialized to null", path.display()));
                (None, true)
            }
            Err(e) => {
                logger::error(format!("Failed to load {}: {e}", path.display()));
                (None, true)
            }
        },
        Err(e) => {
            logger::error(format!("Failed to read {}: {e}", path.display()));
            (None, true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbclient_core::models::{ConnectionConfig, ConnectionType, TabState};

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(AppPaths::at(dir.path()));
        (dir, s)
    }

    fn state(password: &str) -> AppState {
        AppState {
            theme: "Dracula".into(),
            max_rows: 123,
            saved_connections: vec![ConnectionConfig {
                id: "c1".into(),
                display_name: "Conn One".into(),
                kind: ConnectionType::MySql,
                address: "db.example".into(),
                user: "u".into(),
                password: password.into(),
                use_ssh: true,
                ssh_password: format!("ssh-{password}"),
                ssh_key_passphrase: format!("kp-{password}"),
                ..Default::default()
            }],
            open_connection_ids: vec!["c1".into()],
            active_connection_tab_id: Some("tab1".into()),
            ..Default::default()
        }
    }

    #[test]
    fn save_then_load_round_trips_and_never_writes_plaintext() {
        let (_d, s) = store();
        let st = state("s3cret-plain");
        s.save_state(&st).unwrap();
        let file = s.paths.state_file();
        assert!(file.exists());
        assert!(!s.paths.root.join("state.json.tmp").exists());
        let json = std::fs::read_to_string(&file).unwrap();
        assert!(!json.contains("s3cret-plain"));
        assert!(json.contains("\"theme\": \"Dracula\""));
        assert!(json.contains("\"version\": 1"));
        // The live object must not be mutated.
        assert_eq!(st.saved_connections[0].password, "s3cret-plain");

        let loaded = Store::new(AppPaths::at(&s.paths.root)).load_state();
        assert_eq!(loaded.theme, "Dracula");
        assert_eq!(loaded.max_rows, 123);
        assert_eq!(loaded.active_connection_tab_id.as_deref(), Some("tab1"));
        let c = &loaded.saved_connections[0];
        assert_eq!(c.password, "s3cret-plain");
        assert_eq!(c.ssh_password, "ssh-s3cret-plain");
        assert_eq!(c.ssh_key_passphrase, "kp-s3cret-plain");
        assert_eq!(c.kind, ConnectionType::MySql);
    }

    #[test]
    fn second_save_keeps_previous_as_bak() {
        let (_d, s) = store();
        s.save_state(&AppState { theme: "First".into(), ..Default::default() }).unwrap();
        s.save_state(&AppState { theme: "Second".into(), ..Default::default() }).unwrap();
        assert!(std::fs::read_to_string(s.paths.root.join("state.json.bak")).unwrap().contains("\"First\""));
        assert!(std::fs::read_to_string(s.paths.state_file()).unwrap().contains("\"Second\""));
    }

    #[test]
    fn corrupt_file_is_quarantined_and_bak_is_used() {
        let (_d, s) = store();
        s.save_state(&AppState { theme: "Good".into(), ..Default::default() }).unwrap();
        s.save_state(&AppState { theme: "Good".into(), ..Default::default() }).unwrap();
        std::fs::write(s.paths.state_file(), "{ this is not json").unwrap();
        let loaded = s.load_state();
        assert_eq!(loaded.theme, "Good");
        assert!(!s.paths.state_file().exists());
        let q: Vec<_> = std::fs::read_dir(&s.paths.root)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("state.json.corrupt-"))
            .collect();
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn corrupt_file_without_bak_yields_defaults() {
        let (_d, s) = store();
        std::fs::create_dir_all(&s.paths.root).unwrap();
        std::fs::write(s.paths.state_file(), "null").unwrap();
        let loaded = s.load_state();
        assert_eq!(loaded.theme, "Dark");
        assert!(loaded.saved_connections.is_empty());
    }

    #[test]
    fn missing_file_yields_defaults() {
        let (_d, s) = store();
        let loaded = s.load_state();
        assert_eq!(loaded.theme, "Dark");
        assert_eq!(loaded.max_rows, 100_000);
    }

    #[test]
    fn newer_version_file_is_backed_up_before_first_save() {
        let (_d, s) = store();
        std::fs::create_dir_all(&s.paths.root).unwrap();
        std::fs::write(s.paths.state_file(), r#"{ "version": 99, "theme": "Future" }"#).unwrap();
        let loaded = s.load_state();
        assert_eq!(loaded.theme, "Future");
        assert_eq!(loaded.version, 99);
        s.save_state(&loaded).unwrap();
        let newer: Vec<_> = std::fs::read_dir(&s.paths.root)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("state.json.newer-"))
            .collect();
        assert_eq!(newer.len(), 1);
        assert!(std::fs::read_to_string(newer[0].path()).unwrap().contains("\"Future\""));
        assert!(std::fs::read_to_string(s.paths.state_file()).unwrap().contains("\"version\": 1"));
    }

    #[test]
    fn reads_dotnet_integer_enums() {
        let (_d, s) = store();
        std::fs::create_dir_all(&s.paths.root).unwrap();
        std::fs::write(
            s.paths.state_file(),
            r#"{"version":1,"savedConnections":[{"id":"a","displayName":"x","type":1,"authMode":2,"address":"h","useSSH":true}]}"#,
        )
        .unwrap();
        let loaded = s.load_state();
        let c = &loaded.saved_connections[0];
        assert_eq!(c.kind, ConnectionType::MySql);
        assert_eq!(c.auth_mode, dbclient_core::models::SqlAuthMode::Windows);
        assert!(c.use_ssh);
        assert_eq!(c.connection_timeout, 15);
    }

    #[test]
    fn connection_state_round_trip_and_migration() {
        let (_d, s) = store();
        let id = "conn-1";
        s.save_connection_state(&ConnectionTabState {
            id: "t".into(),
            connection_id: id.into(),
            active_database: "main".into(),
            active_query_tab_id: Some("q1".into()),
            query_tabs: vec![TabState { id: "q1".into(), title: "Q".into(), query_text: "SELECT 1".into(), order: 0, database: String::new() }],
            ..Default::default()
        })
        .unwrap();
        let loaded = s.load_connection_state(id).unwrap();
        assert_eq!(loaded.query_tabs[0].database, "main");
        assert_eq!(loaded.active_query_tab_by_database["main"].as_deref(), Some("q1"));
        s.delete_connection_state(id);
        assert!(s.load_connection_state(id).is_none());
    }

    #[test]
    fn history_is_newest_first_capped_and_filterable() {
        let (_d, s) = store();
        for i in 0..105 {
            s.add_history(QueryHistoryEntry {
                query: format!("q{i}"),
                connection_id: if i % 2 == 0 { "a".into() } else { "b".into() },
                database: "db".into(),
                ..Default::default()
            })
            .unwrap();
        }
        let all = s.load_history();
        assert_eq!(all.len(), 100);
        assert_eq!(all[0].query, "q104");
        assert_eq!(all[99].query, "q5");
        assert!(s.history_for_connection(Some("a"), None).iter().all(|e| e.connection_id == "a"));
        assert_eq!(s.history_for_connection(Some("a"), Some("q100")).len(), 1);
        assert_eq!(s.history_for_connection(None, Some("Q104")).len(), 1);
    }

    #[test]
    fn missing_or_corrupt_history_loads_empty() {
        let (_d, s) = store();
        assert!(s.load_history().is_empty());
        std::fs::create_dir_all(&s.paths.root).unwrap();
        std::fs::write(s.paths.history_file(), "not json").unwrap();
        assert!(s.load_history().is_empty());
    }

    #[test]
    fn history_reads_dotnet_pascal_case() {
        let (_d, s) = store();
        std::fs::create_dir_all(&s.paths.root).unwrap();
        std::fs::write(
            s.paths.history_file(),
            r#"[{"Query":"SELECT 1","Database":"d","Connection":"c","ConnectionId":"id","ExecutedAt":"2026-01-02T03:04:05.1234567-05:00"}]"#,
        )
        .unwrap();
        let h = s.load_history();
        assert_eq!(h[0].query, "SELECT 1");
        assert_eq!(h[0].connection_id, "id");
    }
}
