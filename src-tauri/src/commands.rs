//! Tauri commands: thin wrappers over the engine. Every fallible command returns
//! `Result<T, String>` so errors surface as rejected promises in the UI.

use dbclient_core::intellisense::{CompletionItem, Diagnostic, HoverInfo};
use dbclient_core::models::*;
use dbclient_core::update_sql::{EditSet, UpdateScript};
use dbclient_engine::{AppInfo, OpenedConnection};
use serde::Deserialize;
use std::collections::HashSet;
use tauri::State;

use crate::AppState;

type CmdResult<T> = Result<T, String>;

// --- app state / saved connections -------------------------------------------

#[tauri::command]
pub fn get_app_state(state: State<'_, AppState>) -> dbclient_core::models::AppState {
    state.workspace.app_state()
}

#[tauri::command]
pub fn save_app_state(state: State<'_, AppState>, app_state: dbclient_core::models::AppState) -> CmdResult<dbclient_core::models::AppState> {
    state.workspace.save_app_state(app_state)
}

#[tauri::command]
pub fn upsert_connection(state: State<'_, AppState>, config: ConnectionConfig) -> CmdResult<ConnectionConfig> {
    state.workspace.upsert_connection(config)
}

#[tauri::command]
pub fn delete_connection(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.workspace.delete_connection(&id)
}

#[tauri::command]
pub fn load_connection_state(state: State<'_, AppState>, connection_id: String) -> Option<ConnectionTabState> {
    state.workspace.load_connection_state(&connection_id)
}

#[tauri::command]
pub fn save_connection_state(state: State<'_, AppState>, tab_state: ConnectionTabState) -> CmdResult<()> {
    state.workspace.save_connection_state(&tab_state)
}

// --- sessions ------------------------------------------------------------------

#[tauri::command]
pub async fn open_connection(state: State<'_, AppState>, id: String) -> CmdResult<OpenedConnection> {
    state.workspace.open_connection(&id).await
}

#[tauri::command]
pub async fn close_connection(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.workspace.close_connection(&id).await;
    Ok(())
}

#[tauri::command]
pub async fn reload_databases(state: State<'_, AppState>, id: String) -> CmdResult<Vec<String>> {
    state.workspace.reload_databases(&id).await
}

#[tauri::command]
pub async fn load_schema(state: State<'_, AppState>, id: String, database: String, force: bool) -> CmdResult<DbDatabase> {
    state.workspace.load_schema(&id, &database, force).await
}

#[tauri::command]
pub async fn execute_query(state: State<'_, AppState>, id: String, job_id: String, database: String, sql: String) -> CmdResult<QueryResult> {
    state.workspace.execute_query(&id, &job_id, &database, &sql).await
}

#[tauri::command]
pub fn cancel_query(state: State<'_, AppState>, job_id: String) {
    state.workspace.cancel_query(&job_id)
}

#[tauri::command]
pub fn get_completions(state: State<'_, AppState>, id: String, database: String, sql: String, caret: usize) -> Vec<CompletionItem> {
    state.workspace.completions(&id, &database, &sql, caret)
}

#[tauri::command]
pub fn completion_accepted(state: State<'_, AppState>, id: String, text: String) {
    state.workspace.completion_accepted(&id, &text)
}

#[tauri::command]
pub fn get_hover(state: State<'_, AppState>, id: String, database: String, sql: String, caret: usize) -> Option<HoverInfo> {
    state.workspace.hover(&id, &database, &sql, caret)
}

#[tauri::command]
pub fn get_diagnostics(state: State<'_, AppState>, id: String, database: String, sql: String) -> Vec<Diagnostic> {
    state.workspace.diagnostics(&id, &database, &sql)
}

#[tauri::command]
pub async fn test_connection(state: State<'_, AppState>, config: ConnectionConfig) -> CmdResult<TestOutcome> {
    Ok(state.workspace.test_connection(config).await)
}

#[tauri::command]
pub async fn rename_database(state: State<'_, AppState>, id: String, old_name: String, new_name: String) -> CmdResult<Vec<String>> {
    state.workspace.rename_database(&id, &old_name, &new_name).await
}

#[tauri::command]
pub async fn drop_database(state: State<'_, AppState>, id: String, name: String) -> CmdResult<Vec<String>> {
    state.workspace.drop_database(&id, &name).await
}

#[tauri::command]
pub async fn external_connection_string(state: State<'_, AppState>, id: String, database: String) -> CmdResult<String> {
    state.workspace.external_connection_string(&id, &database).await
}

// --- history -------------------------------------------------------------------

#[tauri::command]
pub fn get_history(state: State<'_, AppState>, connection_id: Option<String>, filter: Option<String>) -> Vec<QueryHistoryEntry> {
    state.workspace.history(connection_id.as_deref(), filter.as_deref())
}

#[tauri::command]
pub fn add_history(state: State<'_, AppState>, connection_id: String, database: String, query: String) -> CmdResult<()> {
    state.workspace.add_history(&connection_id, &database, &query)
}

// --- ssh ---------------------------------------------------------------------------

#[tauri::command]
pub fn answer_host_key(state: State<'_, AppState>, request_id: String, trust: bool) {
    state.workspace.answer_host_key(&request_id, trust)
}

// --- SqlPackage ------------------------------------------------------------------

#[tauri::command]
pub fn sqlpackage_path(state: State<'_, AppState>, recheck: bool) -> Option<String> {
    if recheck {
        state.workspace.sqlpackage_recheck()
    } else {
        state.workspace.sqlpackage_path()
    }
}

#[tauri::command]
pub async fn run_sqlpackage(state: State<'_, AppState>, job_id: String, args: Vec<String>) -> CmdResult<Option<i32>> {
    state.workspace.run_sqlpackage(&job_id, args).await
}

#[tauri::command]
pub fn cancel_sqlpackage(state: State<'_, AppState>, job_id: String) {
    state.workspace.cancel_sqlpackage(&job_id)
}

// --- files ---------------------------------------------------------------------------

#[tauri::command]
pub async fn read_text_file(path: String) -> CmdResult<String> {
    tokio::fs::read_to_string(&path).await.map_err(|e| format!("Could not read {path}: {e}"))
}

#[tauri::command]
pub async fn write_text_file(path: String, contents: String) -> CmdResult<()> {
    tokio::fs::write(&path, contents).await.map_err(|e| format!("Could not write {path}: {e}"))
}

#[tauri::command]
pub fn file_exists(path: String) -> bool {
    std::path::Path::new(&path).is_file()
}

// --- misc ------------------------------------------------------------------------------

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    state.workspace.app_info()
}

#[tauri::command]
pub fn format_sql(sql: String) -> String {
    dbclient_core::formatter::format(&sql)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateScriptRequest {
    pub table_name: String,
    pub schema: Option<String>,
    pub connection_type: ConnectionType,
    pub pk_columns: Vec<String>,
    pub column_names: Vec<String>,
    pub column_types: Vec<Option<String>>,
    pub original_rows: Vec<Vec<Option<String>>>,
    pub current_rows: Vec<Vec<Option<String>>>,
    pub dirty_rows: Vec<usize>,
    pub query_text: String,
}

#[tauri::command]
pub fn generate_update_script(request: UpdateScriptRequest) -> UpdateScript {
    let pks: HashSet<String> = request.pk_columns.iter().cloned().collect();
    dbclient_core::update_sql::generate(&EditSet {
        table_name: &request.table_name,
        schema: request.schema.as_deref(),
        connection_type: request.connection_type,
        pk_columns: &pks,
        column_names: &request.column_names,
        column_types: &request.column_types,
        original_rows: &request.original_rows,
        current_rows: &request.current_rows,
        dirty_rows: &request.dirty_rows,
        query_text: &request.query_text,
    })
}
