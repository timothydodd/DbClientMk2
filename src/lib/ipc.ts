import { invoke } from "@tauri-apps/api/core";
import type {
  AppInfo,
  AppState,
  CompletionItem,
  ConnectionConfig,
  ConnectionTabState,
  DbDatabase,
  HoverInfo,
  SqlDiagnostic,
  OpenedConnection,
  QueryHistoryEntry,
  QueryResult,
  TestOutcome,
  UpdateScript,
  UpdateScriptRequest,
} from "./types";

// --- app state / saved connections ---
export const getAppState = () => invoke<AppState>("get_app_state");
export const saveAppState = (appState: AppState) => invoke<AppState>("save_app_state", { appState });
export const upsertConnection = (config: ConnectionConfig) => invoke<ConnectionConfig>("upsert_connection", { config });
export const deleteConnection = (id: string) => invoke<void>("delete_connection", { id });
export const loadConnectionState = (connectionId: string) => invoke<ConnectionTabState | null>("load_connection_state", { connectionId });
export const saveConnectionState = (tabState: ConnectionTabState) => invoke<void>("save_connection_state", { tabState });

// --- sessions ---
export const openConnection = (id: string) => invoke<OpenedConnection>("open_connection", { id });
export const closeConnection = (id: string) => invoke<void>("close_connection", { id });
export const reloadDatabases = (id: string) => invoke<string[]>("reload_databases", { id });
export const loadSchema = (id: string, database: string, force: boolean) => invoke<DbDatabase>("load_schema", { id, database, force });
export const executeQuery = (id: string, jobId: string, database: string, sql: string) =>
  invoke<QueryResult>("execute_query", { id, jobId, database, sql });
export const cancelQuery = (jobId: string) => invoke<void>("cancel_query", { jobId });
export const getCompletions = (id: string, database: string, sql: string, caret: number) =>
  invoke<CompletionItem[]>("get_completions", { id, database, sql, caret });
export const completionAccepted = (id: string, text: string) => invoke<void>("completion_accepted", { id, text });
export const getHover = (id: string, database: string, sql: string, caret: number) => invoke<HoverInfo | null>("get_hover", { id, database, sql, caret });
export const getDiagnostics = (id: string, database: string, sql: string) => invoke<SqlDiagnostic[]>("get_diagnostics", { id, database, sql });
export const testConnection = (config: ConnectionConfig) => invoke<TestOutcome>("test_connection", { config });
export const renameDatabase = (id: string, oldName: string, newName: string) => invoke<string[]>("rename_database", { id, oldName, newName });
export const dropDatabase = (id: string, name: string) => invoke<string[]>("drop_database", { id, name });
export const externalConnectionString = (id: string, database: string) => invoke<string>("external_connection_string", { id, database });

// --- history ---
export const getHistory = (connectionId: string | null, filter: string | null) =>
  invoke<QueryHistoryEntry[]>("get_history", { connectionId, filter });
export const addHistory = (connectionId: string, database: string, query: string) => invoke<void>("add_history", { connectionId, database, query });

// --- ssh ---
export const answerHostKey = (requestId: string, trust: boolean) => invoke<void>("answer_host_key", { requestId, trust });

// --- SqlPackage ---
export const sqlpackagePath = (recheck = false) => invoke<string | null>("sqlpackage_path", { recheck });
export const runSqlpackage = (jobId: string, args: string[]) => invoke<number | null>("run_sqlpackage", { jobId, args });
export const cancelSqlpackage = (jobId: string) => invoke<void>("cancel_sqlpackage", { jobId });

// --- files / misc ---
export const readTextFile = (path: string) => invoke<string>("read_text_file", { path });
export const writeTextFile = (path: string, contents: string) => invoke<void>("write_text_file", { path, contents });
export const fileExists = (path: string) => invoke<boolean>("file_exists", { path });
export const appInfo = () => invoke<AppInfo>("app_info");
export const formatSql = (sql: string) => invoke<string>("format_sql", { sql });
export const generateUpdateScript = (request: UpdateScriptRequest) => invoke<UpdateScript>("generate_update_script", { request });
