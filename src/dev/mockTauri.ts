// Dev-only stand-in for the Tauri backend so the UI can be viewed in a plain browser
// (`npm run dev`, open http://localhost:1420). Loaded by main.tsx only without a Tauri runtime.

import type { AppState, ConnectionConfig, DbDatabase, QueryResult } from "../lib/types";

const conn: ConnectionConfig = {
  id: "demo",
  displayName: "Demo (SQLite)",
  type: "sqlite",
  address: "",
  user: "",
  password: "",
  port: "",
  database: "",
  authMode: "sqlLogin",
  fileName: "C:\\data\\demo.db",
  connectionTimeout: 15,
  commandTimeout: 30,
  useSSH: false,
  sshHost: "",
  sshUser: "",
  sshPassword: "",
  sshRemotePort: 0,
  sshKeyFile: "",
  sshKeyPassphrase: "",
  encrypt: true,
  trustServerCertificate: false,
};

let appState: AppState = {
  version: 1,
  savedConnections: [conn],
  openConnectionIds: ["demo"],
  activeConnectionTabId: "demo",
  isConnectionPanelOpen: true,
  isHistoryPanelOpen: false,
  theme: "Dark",
  maxRows: 100000,
  editorFontSize: 14,
  editorWordWrap: false,
  leftPanelWidth: 260,
  editorHeightRatio: 0.55,
  historyPanelWidth: 280,
};

const schema: DbDatabase = {
  name: "demo.db",
  loaded: true,
  tables: [
    {
      name: "people",
      schema: "",
      columns: [
        { name: "id", dataType: "INTEGER", isPrimaryKey: true, isNullable: false },
        { name: "name", dataType: "TEXT", isPrimaryKey: false, isNullable: false },
        { name: "email", dataType: "TEXT", isPrimaryKey: false, isNullable: true },
        { name: "created_at", dataType: "DATETIME", isPrimaryKey: false, isNullable: false },
      ],
    },
    { name: "orders", schema: "", columns: [{ name: "id", dataType: "INTEGER", isPrimaryKey: true, isNullable: false }, { name: "person_id", dataType: "INTEGER", isPrimaryKey: false, isNullable: false }, { name: "total", dataType: "REAL", isPrimaryKey: false, isNullable: true }] },
  ],
  views: [{ name: "v_people", schema: "", columns: [{ name: "name", dataType: "TEXT", isPrimaryKey: false, isNullable: false }] }],
  storedProcedures: [],
  functions: [],
  foreignKeys: [{ name: "fk_orders_people", schema: "", table: "orders", columns: ["person_id"], refSchema: "", refTable: "people", refColumns: ["id"] }],
};

function demoResult(sql: string): QueryResult {
  if (/error/i.test(sql)) return { data: [], affectedRows: 0, errorMessage: "no such table: nope", errorCode: 1, sqlState: null, errorLine: 1, executionMs: 3, messages: [], cancelled: false };
  const rows = Array.from({ length: 500 }, (_, i) => [String(i + 1), `Person ${i + 1}`, i % 7 === 0 ? null : `p${i + 1}@example.com`, "2026-01-02 03:04:05"]);
  return {
    data: [{ columnNames: ["id", "name", "email", "created_at"], columnTypes: ["INTEGER", "TEXT", "TEXT", "DATETIME"], rows, truncated: false }],
    affectedRows: rows.length,
    errorMessage: null,
    errorCode: null,
    sqlState: null,
    errorLine: null,
    executionMs: 12,
    messages: [],
    cancelled: false,
  };
}

const handlers: Record<string, (a: Record<string, unknown>) => unknown> = {
  get_app_state: () => appState,
  save_app_state: (a) => (appState = a.appState as AppState),
  upsert_connection: (a) => a.config,
  delete_connection: () => null,
  load_connection_state: () => null,
  save_connection_state: () => null,
  open_connection: () => ({ databases: ["demo.db"] }),
  close_connection: () => null,
  reload_databases: () => ["demo.db"],
  load_schema: () => schema,
  execute_query: (a) => new Promise((r) => setTimeout(() => r(demoResult(a.sql as string)), 300)),
  cancel_query: () => null,
  get_completions: () => [
    { text: "people", type: "table", priority: 1, description: "people", score: 50 },
    { text: "orders", type: "table", priority: 1, description: "orders", score: 50 },
    { text: "JOIN orders ON person_id", type: "join", priority: 0, description: "fk", insertText: "INNER JOIN orders o ON o.person_id = p.id", score: 50 },
    { text: "ssf", type: "snippet", priority: 0, description: "SELECT * FROM", insertText: "SELECT * FROM $0", score: 50 },
    { text: "SELECT", type: "keyword", priority: 5, description: null, score: 50 },
  ],
  completion_accepted: () => null,
  get_hover: () => ({ title: "people.name", detail: "TEXT NOT NULL\npeople", start: 0, end: 4 }),
  get_diagnostics: () => [],
  test_connection: () => ({ ok: true, message: "Connection successful!" }),
  get_history: () => [{ query: "SELECT * FROM people", database: "demo.db", connection: "Demo", connectionId: "demo", executedAt: new Date().toISOString() }],
  add_history: () => null,
  sqlpackage_path: () => null,
  app_info: () => ({ version: "dev", dataDir: "C:\\Users\\me\\.dbclient", logFile: "C:\\Users\\me\\.dbclient\\log.txt", os: "browser", sqlpackagePath: null }),
  format_sql: (a) => String(a.sql).toUpperCase(),
  generate_update_script: () => ({ sql: "UPDATE \"people\" SET \"name\" = 'x' WHERE \"id\" = 1;", expectedStatements: 1, error: null }),
  read_text_file: () => "SELECT 1",
  write_text_file: () => null,
  file_exists: () => true,
  external_connection_string: () => "Data Source=.;Initial Catalog=demo",
  rename_database: () => ["demo.db"],
  drop_database: () => [],
  answer_host_key: () => null,
  run_sqlpackage: () => 0,
  cancel_sqlpackage: () => null,
};

interface Internals {
  invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
  transformCallback: (cb?: (r: unknown) => void) => number;
  metadata: { currentWindow: { label: string }; currentWebview: { label: string } };
  plugins: Record<string, unknown>;
}

(window as unknown as { __TAURI_INTERNALS__: Internals }).__TAURI_INTERNALS__ = {
  invoke: async (cmd, args = {}) => {
    if (cmd.startsWith("plugin:event|")) return 1;
    if (cmd.startsWith("plugin:window|")) return false;
    if (cmd.startsWith("plugin:")) return null;
    const h = handlers[cmd];
    if (!h) throw new Error(`mock: unknown command ${cmd}`);
    return h(args);
  },
  transformCallback: () => Math.floor(Math.random() * 1e9),
  metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  plugins: {},
};
