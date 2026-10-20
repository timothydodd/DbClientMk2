// Hand-mirrored from the Rust DTOs (crates/dbclient-core/src/models.rs, intellisense/mod.rs,
// dbclient-engine/src/lib.rs). Keep in sync.

export type ConnectionType = "sqlServer" | "mySql" | "sqlite";
export type SqlAuthMode = "sqlLogin" | "azureDefault" | "windows";

/** What the backend sends instead of a saved secret; send it back to keep the saved value. */
export const SAVED_SECRET = "\u0000saved\u0000";

export interface ConnectionConfig {
  id: string;
  displayName: string;
  type: ConnectionType;
  address: string;
  user: string;
  password: string;
  port: string;
  database: string;
  authMode: SqlAuthMode;
  fileName: string;
  connectionTimeout: number;
  commandTimeout: number;
  useSSH: boolean;
  sshHost: string;
  sshUser: string;
  sshPassword: string;
  sshRemotePort: number;
  sshKeyFile: string;
  sshKeyPassphrase: string;
  encrypt: boolean;
  trustServerCertificate: boolean;
}

export type ThemeName = "Dark" | "Dracula" | "Light";

export interface AppState {
  version: number;
  savedConnections: ConnectionConfig[];
  openConnectionIds: string[];
  activeConnectionTabId: string | null;
  isConnectionPanelOpen: boolean;
  isHistoryPanelOpen: boolean;
  theme: string;
  maxRows: number;
  editorFontSize: number;
  editorWordWrap: boolean;
  leftPanelWidth: number | null;
  editorHeightRatio: number | null;
  historyPanelWidth: number | null;
}

export interface TabState {
  id: string;
  title: string;
  queryText: string;
  order: number;
  database: string;
}

export interface ConnectionTabState {
  id: string;
  connectionId: string;
  activeDatabase: string;
  queryTabs: TabState[];
  activeQueryTabId: string | null;
  activeQueryTabByDatabase: Record<string, string | null>;
}

export interface QueryHistoryEntry {
  query: string;
  database: string;
  connection: string;
  connectionId: string;
  executedAt: string;
}

export interface DbColumn {
  name: string;
  dataType: string;
  isPrimaryKey: boolean;
  isNullable: boolean;
}

export interface DbTable {
  name: string;
  schema: string;
  columns: DbColumn[];
}

export interface DbParameter {
  name: string;
  dataType: string;
  isOutput: boolean;
}

export interface DbProc {
  name: string;
  schema: string;
  parameters: DbParameter[];
  returns: string;
}

export interface DbForeignKey {
  name: string;
  schema: string;
  table: string;
  columns: string[];
  refSchema: string;
  refTable: string;
  refColumns: string[];
}

export interface DbDatabase {
  name: string;
  loaded: boolean;
  tables: DbTable[];
  views: DbTable[];
  storedProcedures: DbProc[];
  functions: DbProc[];
  foreignKeys: DbForeignKey[];
}

export type Cell = string | null;

export interface ResultSet {
  columnNames: string[];
  columnTypes: (string | null)[];
  rows: Cell[][];
  truncated: boolean;
}

export interface QueryResult {
  data: ResultSet[];
  affectedRows: number;
  errorMessage: string | null;
  errorCode: number | null;
  sqlState: string | null;
  errorLine: number | null;
  executionMs: number;
  messages: string[];
  cancelled: boolean;
}

export interface TestOutcome {
  ok: boolean;
  message: string;
}

export type CompletionType =
  | "keyword"
  | "table"
  | "view"
  | "column"
  | "alias"
  | "schema"
  | "cte"
  | "function"
  | "procedure"
  | "variable"
  | "snippet"
  | "join"
  | "expansion"
  | "value";

export interface CompletionItem {
  text: string;
  type: CompletionType;
  priority: number;
  description: string | null;
  /** Text to insert when it differs from `text`; may contain `$0` / `${n:placeholder}` markers. */
  insertText?: string;
  /** Char offset to replace from, when the item replaces more than the typed word. */
  replaceFrom?: number;
  score: number;
}

export interface HoverInfo {
  title: string;
  detail: string;
  start: number;
  end: number;
}

export interface SqlDiagnostic {
  start: number;
  end: number;
  message: string;
  severity: "error" | "warning";
}

export interface OpenedConnection {
  databases: string[];
}

export interface AppInfo {
  version: string;
  dataDir: string;
  logFile: string;
  os: string;
  sqlpackagePath: string | null;
}

export interface UpdateScript {
  sql: string;
  expectedStatements: number;
  error: string | null;
}

export interface UpdateScriptRequest {
  tableName: string;
  schema: string | null;
  connectionType: ConnectionType;
  pkColumns: string[];
  columnNames: string[];
  columnTypes: (string | null)[];
  originalRows: Cell[][];
  currentRows: Cell[][];
  dirtyRows: number[];
  queryText: string;
}

export interface HostKeyEvent {
  requestId: string;
  host: string;
  port: number;
  keyType: string;
  fingerprintSha256: string;
  fingerprintMd5: string;
}

export interface SqlPackageLine {
  jobId: string;
  line: string;
}

export const CONNECTION_TYPE_LABEL: Record<ConnectionType, string> = {
  sqlServer: "SQL Server",
  mySql: "MySQL",
  sqlite: "SQLite",
};

export function connectionTitle(c: ConnectionConfig): string {
  if (c.displayName) return c.displayName;
  if (c.type === "sqlite") return c.fileName.split(/[\\/]/).pop() ?? "";
  return `${CONNECTION_TYPE_LABEL[c.type]}: ${c.address}`;
}

export function defaultConnection(): ConnectionConfig {
  return {
    id: "",
    displayName: "",
    type: "sqlServer",
    address: "",
    user: "",
    password: "",
    port: "",
    database: "",
    authMode: "sqlLogin",
    fileName: "",
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
}
