// The workspace: saved connections, open connection tabs (each with per-database query tab
// sets), schema trees, query execution, layout and editor settings, persistence.

import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import * as ipc from "../lib/ipc";
import { errorMessage, fileName, fileStem, fmtCount, fmtMs } from "../lib/format";
import { appendStatement, explain as explainSql } from "../lib/sql";
import {
  activateDatabaseTabs,
  activeTabIdsByDatabase,
  allTabs,
  collectAllTabStates,
  dropTabSet,
  emptySets,
  renameTabSet,
  restoreTabsForDatabase,
  type TabSets,
} from "../lib/tabs";
import { connectionTitle, type AppState, type ConnectionConfig, type DbDatabase, type HostKeyEvent, type QueryResult, type ThemeName } from "../lib/types";
import { normalizeTheme, useThemeStore } from "./theme";
import { toast } from "./toast";
import { ui, useUiStore } from "./ui";

export type MessageTone = "info" | "error" | "warning" | "success";

export interface QueryTab {
  id: string;
  title: string;
  database: string;
  queryText: string;
  result: QueryResult | null;
  selectedResultIndex: number;
  executing: boolean;
  message: { text: string; tone: MessageTone } | null;
  rowCountText: string;
  statusText: string;
  executionTimeText: string;
  cursorLine: number;
  cursorColumn: number;
  /** The text that produced `result` (selection or whole editor). */
  executedText: string;
}

export interface ConnectionTab {
  /** Session id (persisted in the connection's state file). */
  id: string;
  config: ConnectionConfig;
  activeDatabase: string;
  databases: string[];
  sets: TabSets<QueryTab>;
  counter: number;
  connected: boolean;
  connecting: boolean;
  schemaLoading: boolean;
  connectionError: string | null;
  statusText: string;
  schema: DbDatabase | null;
  treeFilter: string;
  filterTables: boolean;
  filterColumns: boolean;
}

/** What the editor exposes to the store (selection, replace text, line-comment toggle). */
export interface EditorApi {
  getText: () => string;
  getSelection: () => string;
  setText: (text: string) => void;
  toggleComment: () => void;
  focus: () => void;
  openSearch: () => void;
}

const editors = new Map<string, EditorApi>();
export const registerEditor = (tabId: string, api: EditorApi | null) => {
  if (api) editors.set(tabId, api);
  else editors.delete(tabId);
};
export const getEditor = (tabId: string) => editors.get(tabId);

export const MIN_FONT = 8;
export const MAX_FONT = 40;
export const DEFAULT_FONT = 14;

let autosaveTimer: ReturnType<typeof setTimeout> | null = null;
let initialized = false;
let historyTick = 0;

const uid = () => (crypto.randomUUID?.() ?? `${Date.now().toString(16)}${Math.random().toString(16).slice(2)}`).replace(/-/g, "");

function newTab(title: string, database: string, text = "", id = uid()): QueryTab {
  return {
    id,
    title,
    database,
    queryText: text,
    result: null,
    selectedResultIndex: 0,
    executing: false,
    message: null,
    rowCountText: "",
    statusText: "",
    executionTimeText: "",
    cursorLine: 1,
    cursorColumn: 1,
    executedText: "",
  };
}

function connectionTab(config: ConnectionConfig, id: string): ConnectionTab {
  return {
    id,
    config,
    activeDatabase: "",
    databases: [],
    sets: emptySets(),
    counter: 0,
    connected: false,
    connecting: false,
    schemaLoading: false,
    connectionError: null,
    statusText: "Connecting...",
    schema: null,
    treeFilter: "",
    filterTables: true,
    filterColumns: false,
  };
}

interface WorkspaceState {
  appState: AppState | null;
  connections: ConnectionTab[];
  selectedConnectionId: string | null;
  /** Bumped when history changes so panels re-query. */
  historyTick: number;
  init: () => Promise<void>;

  // saved connections
  upsertConnection: (config: ConnectionConfig) => Promise<ConnectionConfig>;
  deleteSavedConnection: (id: string) => Promise<void>;

  // connection tabs
  openConnection: (configId: string) => Promise<void>;
  closeConnection: (configId: string) => Promise<void>;
  retryConnection: (configId: string) => Promise<void>;
  selectConnection: (configId: string) => void;
  switchDatabase: (configId: string, database: string, force?: boolean) => Promise<void>;
  reloadDatabases: (configId: string) => Promise<void>;
  renameDatabase: (configId: string, oldName: string, newName: string) => Promise<string | null>;
  dropDatabase: (configId: string, name: string) => Promise<string | null>;
  setTreeFilter: (configId: string, filter: string) => void;
  setFilterScope: (configId: string, tables: boolean, columns: boolean) => void;

  // query tabs
  newQueryTab: (configId?: string, title?: string, text?: string) => QueryTab | null;
  closeQueryTab: (configId: string, tabId: string) => Promise<void>;
  closeOtherTabs: (configId: string, tabId: string) => Promise<void>;
  closeTabsToRight: (configId: string, tabId: string) => Promise<void>;
  closeTabsToLeft: (configId: string, tabId: string) => Promise<void>;
  renameTab: (configId: string, tabId: string, title: string) => void;
  moveTab: (configId: string, from: number, to: number) => void;
  selectQueryTab: (configId: string, tabId: string) => void;
  nextQueryTab: (direction: 1 | -1) => void;
  setQueryText: (configId: string, tabId: string, text: string) => void;
  setCursor: (configId: string, tabId: string, line: number, column: number) => void;
  setSelectedResult: (configId: string, tabId: string, index: number) => void;
  showMessage: (configId: string, tabId: string, text: string, tone: MessageTone) => void;
  insertSql: (configId: string, sql: string) => void;
  /** Replace the active tab's text (editor syncs). */
  setActiveText: (text: string) => void;

  // execution
  execute: (sqlOverride?: string) => Promise<void>;
  executeOn: (configId: string, tabId: string, sql: string, database?: string) => Promise<QueryResult | null>;
  cancel: (tabId?: string) => void;
  explain: () => Promise<void>;
  formatCurrent: () => Promise<void>;
  toggleComment: () => void;

  // settings / layout
  setTheme: (theme: ThemeName) => void;
  cycleTheme: () => void;
  setMaxRows: (n: number) => void;
  zoom: (delta: number | null) => void;
  toggleWordWrap: () => void;
  toggleConnectionPanel: () => void;
  toggleHistoryPanel: () => void;
  setPanelOpen: (panel: "connection" | "history", open: boolean) => void;
  setLayout: (patch: Partial<Pick<AppState, "leftPanelWidth" | "editorHeightRatio" | "historyPanelWidth">>) => void;

  // files
  importFile: (path: string) => Promise<void>;
  exportFile: (path: string) => Promise<void>;

  // persistence
  saveState: () => Promise<void>;
  scheduleAutosave: () => void;
  bumpHistory: () => void;
}

export const useWorkspace = create<WorkspaceState>((set, get) => {
  const conn = (configId: string) => get().connections.find((c) => c.config.id === configId);
  const selected = () => {
    const id = get().selectedConnectionId;
    return id ? conn(id) : undefined;
  };
  const activeTab = (c: ConnectionTab | undefined) => c?.sets.tabs.find((t) => t.id === c.sets.selectedTabId) ?? null;

  /** Replace one connection with `fn(copy)`. */
  const update = (configId: string, fn: (c: ConnectionTab) => unknown) =>
    set((s) => ({
      connections: s.connections.map((c) => {
        if (c.config.id !== configId) return c;
        const copy = { ...c };
        fn(copy);
        return copy;
      }),
    }));
  const updateTab = (configId: string, tabId: string, fn: (t: QueryTab) => unknown) =>
    update(configId, (c) => {
      const patch = (t: QueryTab) => {
        if (t.id !== tabId) return t;
        const copy = { ...t };
        fn(copy);
        return copy;
      };
      const stashed: Record<string, QueryTab[]> = {};
      for (const [k, v] of Object.entries(c.sets.stashed)) stashed[k] = v.map(patch);
      c.sets = { ...c.sets, tabs: c.sets.tabs.map(patch), stashed };
    });

  const addTab = (c: ConnectionTab, title?: string, text = ""): QueryTab => {
    c.counter += 1;
    const tab = newTab(title ?? `Query ${c.counter}`, c.sets.tabsActiveDb || c.activeDatabase, text);
    c.sets = { ...c.sets, tabs: [...c.sets.tabs, tab], selectedTabId: tab.id };
    return tab;
  };

  const confirmClose = async (t: QueryTab) => !t.queryText.trim() || (await ui.confirm("Close Tab", `Close '${t.title}'? Its query will be lost.`, { okText: "Close" }));

  const closeTabs = async (configId: string, ids: string[], keep: string) => {
    const c = conn(configId);
    if (!c) return;
    for (const id of ids) {
      const t = c.sets.tabs.find((x) => x.id === id);
      if (!t || !(await confirmClose(t))) continue;
      get().cancel(id);
      editors.delete(id);
      update(configId, (cc) => {
        cc.sets = { ...cc.sets, tabs: cc.sets.tabs.filter((x) => x.id !== id) };
      });
    }
    update(configId, (cc) => {
      cc.sets = { ...cc.sets, selectedTabId: cc.sets.tabs.some((t) => t.id === keep) ? keep : (cc.sets.tabs[0]?.id ?? null) };
      if (cc.sets.tabs.length === 0) addTab(cc);
    });
    get().scheduleAutosave();
  };

  const connectionStateOf = (c: ConnectionTab) => ({
    id: c.id,
    connectionId: c.config.id,
    activeDatabase: c.activeDatabase,
    activeQueryTabId: c.sets.selectedTabId,
    activeQueryTabByDatabase: activeTabIdsByDatabase(c.sets),
    queryTabs: collectAllTabStates(c.sets),
  });

  /** Build a connection tab from saved config + session state, without connecting. */
  const restoreTab = async (config: ConnectionConfig): Promise<ConnectionTab> => {
    const saved = await ipc.loadConnectionState(config.id).catch(() => null);
    const c = connectionTab(config, saved?.id ?? uid());
    if (saved) {
      c.activeDatabase = saved.activeDatabase;
      const byDb = new Map<string, typeof saved.queryTabs>();
      for (const t of saved.queryTabs) {
        const k = t.database ?? "";
        byDb.set(k, [...(byDb.get(k) ?? []), t]);
      }
      let sets = c.sets;
      for (const [db, tabs] of byDb) {
        const ordered = [...tabs].sort((a, b) => a.order - b.order);
        sets = restoreTabsForDatabase(
          sets,
          db,
          ordered.map((t) => newTab(t.title, db, t.queryText, t.id)),
          saved.activeQueryTabByDatabase[db] ?? null,
        );
      }
      c.sets = sets;
      c.counter = saved.queryTabs.length;
      if (c.activeDatabase) c.sets = activateDatabaseTabs(c.sets, c.activeDatabase);
      if (c.sets.tabs.length === 0 && saved.queryTabs.length === 0) addTab(c);
      else if (c.sets.tabs.length === 0 && !c.activeDatabase && Object.keys(c.sets.stashed).length === 0) addTab(c);
    } else {
      addTab(c);
    }
    return c;
  };

  /** Connect: list databases, then load the first/saved database's schema. */
  const connect = async (configId: string) => {
    const c = conn(configId);
    if (!c) return;
    update(configId, (cc) => {
      cc.connecting = true;
      cc.connected = false;
      cc.connectionError = null;
      cc.schemaLoading = true;
      cc.statusText = `Connecting to ${connectionTitle(cc.config)}...`;
    });
    try {
      const opened = await ipc.openConnection(configId);
      update(configId, (cc) => {
        cc.databases = opened.databases;
        cc.connected = true;
        cc.connecting = false;
      });
      const cur = conn(configId)!;
      const target = cur.config.database || cur.activeDatabase || opened.databases[0] || "";
      if (target) await get().switchDatabase(configId, target, true);
      else update(configId, (cc) => (cc.statusText = `${connectionTitle(cc.config)}: no databases`));
    } catch (e) {
      update(configId, (cc) => {
        cc.connecting = false;
        cc.connected = false;
        cc.connectionError = errorMessage(e);
        cc.statusText = `Connection failed: ${errorMessage(e)}`;
      });
    } finally {
      update(configId, (cc) => (cc.schemaLoading = false));
    }
  };

  return {
    appState: null,
    connections: [],
    selectedConnectionId: null,
    historyTick: 0,

    init: async () => {
      if (initialized) return;
      initialized = true;
      await listen<HostKeyEvent>("ssh:host-key", (e) => useUiStore.getState().pushHostKey(e.payload));
      const appState = await ipc.getAppState();
      useThemeStore.getState().setTheme(normalizeTheme(appState.theme));
      set({ appState });
      // Restore open connections from their state files, then connect each.
      const tabs: ConnectionTab[] = [];
      for (const id of appState.openConnectionIds) {
        const config = appState.savedConnections.find((c) => c.id === id);
        if (config) tabs.push(await restoreTab(config));
      }
      set({
        connections: tabs,
        selectedConnectionId: tabs.find((t) => t.config.id === appState.activeConnectionTabId)?.config.id ?? tabs[0]?.config.id ?? null,
      });
      if (tabs.length === 0 && !appState.isConnectionPanelOpen) get().setPanelOpen("connection", true);
      await Promise.all(tabs.map((t) => connect(t.config.id)));
    },

    // --- saved connections ---
    upsertConnection: async (config) => {
      const saved = await ipc.upsertConnection(config);
      set((s) => {
        if (!s.appState) return {};
        const list = s.appState.savedConnections.some((c) => c.id === saved.id)
          ? s.appState.savedConnections.map((c) => (c.id === saved.id ? saved : c))
          : [...s.appState.savedConnections, saved];
        return {
          appState: { ...s.appState, savedConnections: list },
          connections: s.connections.map((c) => (c.config.id === saved.id ? { ...c, config: saved } : c)),
        };
      });
      return saved;
    },
    deleteSavedConnection: async (id) => {
      await ipc.deleteConnection(id);
      set((s) => (s.appState ? { appState: { ...s.appState, savedConnections: s.appState.savedConnections.filter((c) => c.id !== id) } } : {}));
    },

    // --- connection tabs ---
    openConnection: async (configId) => {
      if (conn(configId)) {
        set({ selectedConnectionId: configId });
        return;
      }
      const config = get().appState?.savedConnections.find((c) => c.id === configId);
      if (!config) return;
      const tab = await restoreTab(config);
      set((s) => ({ connections: [...s.connections, tab], selectedConnectionId: configId }));
      await connect(configId);
      await get().saveState();
    },
    closeConnection: async (configId) => {
      const c = conn(configId);
      if (!c) return;
      // No prompt: the connection's query tabs are persisted and come back on reconnect.
      await ipc.saveConnectionState(connectionStateOf(c)).catch(() => undefined);
      for (const t of allTabs(c.sets)) {
        get().cancel(t.id);
        editors.delete(t.id);
      }
      set((s) => {
        const idx = s.connections.findIndex((x) => x.config.id === configId);
        const rest = s.connections.filter((x) => x.config.id !== configId);
        const next = s.selectedConnectionId === configId ? (rest[Math.min(Math.max(idx, 0), rest.length - 1)]?.config.id ?? null) : s.selectedConnectionId;
        return { connections: rest, selectedConnectionId: next };
      });
      if (get().connections.length === 0) get().setPanelOpen("connection", true);
      await ipc.closeConnection(configId).catch(() => undefined);
      await get().saveState();
    },
    retryConnection: (configId) => connect(configId),
    selectConnection: (configId) => {
      set({ selectedConnectionId: configId });
      const c = conn(configId);
      if (c && !c.sets.selectedTabId && c.sets.tabs.length) update(configId, (cc) => (cc.sets = { ...cc.sets, selectedTabId: cc.sets.tabs[0].id }));
      get().scheduleAutosave();
    },
    switchDatabase: async (configId, database, force = false) => {
      const c = conn(configId);
      if (!c || !c.connected || (!force && database === c.activeDatabase)) return;
      update(configId, (cc) => {
        cc.sets = activateDatabaseTabs(cc.sets, database);
        if (cc.sets.tabs.length === 0) addTab(cc);
        cc.activeDatabase = database;
        cc.statusText = `Loading ${database}...`;
        cc.schemaLoading = true;
      });
      try {
        const schema = await ipc.loadSchema(configId, database, force);
        update(configId, (cc) => {
          if (cc.activeDatabase !== database) return;
          cc.schema = schema;
          cc.statusText = `${connectionTitle(cc.config)}: ${database}`;
        });
      } catch (e) {
        update(configId, (cc) => {
          cc.schema = null;
          cc.statusText = `Schema load failed: ${errorMessage(e)}`;
        });
        toast.error(`Schema load failed: ${errorMessage(e)}`);
      } finally {
        update(configId, (cc) => (cc.schemaLoading = false));
      }
      get().scheduleAutosave();
    },
    reloadDatabases: async (configId) => {
      update(configId, (cc) => (cc.schemaLoading = true));
      try {
        const databases = await ipc.reloadDatabases(configId);
        update(configId, (cc) => (cc.databases = databases));
      } catch (e) {
        update(configId, (cc) => (cc.statusText = `Reload failed: ${errorMessage(e)}`));
      } finally {
        update(configId, (cc) => (cc.schemaLoading = false));
      }
    },
    renameDatabase: async (configId, oldName, newName) => {
      try {
        const databases = await ipc.renameDatabase(configId, oldName, newName);
        const wasActive = conn(configId)?.activeDatabase.toLowerCase() === oldName.toLowerCase();
        update(configId, (cc) => {
          cc.databases = databases;
          cc.sets = renameTabSet(cc.sets, oldName, newName);
          if (wasActive) cc.activeDatabase = newName;
          cc.statusText = `Renamed ${oldName} to ${newName}`;
        });
        if (wasActive) await get().switchDatabase(configId, newName, true);
        get().scheduleAutosave();
        return null;
      } catch (e) {
        return errorMessage(e);
      }
    },
    dropDatabase: async (configId, name) => {
      try {
        const databases = await ipc.dropDatabase(configId, name);
        const wasActive = conn(configId)?.activeDatabase.toLowerCase() === name.toLowerCase();
        update(configId, (cc) => {
          cc.databases = databases;
          cc.sets = dropTabSet(cc.sets, name);
          cc.statusText = `Deleted ${name}`;
          if (wasActive) {
            cc.activeDatabase = "";
            cc.schema = null;
          }
        });
        if (wasActive && databases[0]) await get().switchDatabase(configId, databases[0], true);
        get().scheduleAutosave();
        return null;
      } catch (e) {
        return errorMessage(e);
      }
    },
    setTreeFilter: (configId, filter) => update(configId, (cc) => (cc.treeFilter = filter)),
    setFilterScope: (configId, tables, columns) =>
      update(configId, (cc) => {
        cc.filterTables = tables;
        cc.filterColumns = columns;
      }),

    // --- query tabs ---
    newQueryTab: (configId, title, text) => {
      const c = configId ? conn(configId) : selected();
      if (!c) return null;
      let created: QueryTab | null = null;
      update(c.config.id, (cc) => {
        created = addTab(cc, title, text);
      });
      get().scheduleAutosave();
      return created;
    },
    closeQueryTab: async (configId, tabId) => {
      const c = conn(configId);
      if (!c) return;
      const idx = c.sets.tabs.findIndex((t) => t.id === tabId);
      const keep = c.sets.tabs[Math.min(Math.max(idx, 0) + (idx === c.sets.tabs.length - 1 ? -1 : 1), c.sets.tabs.length - 1)]?.id ?? "";
      await closeTabs(configId, [tabId], c.sets.selectedTabId === tabId ? keep : (c.sets.selectedTabId ?? keep));
    },
    closeOtherTabs: async (configId, tabId) => {
      const c = conn(configId);
      if (c) await closeTabs(configId, c.sets.tabs.filter((t) => t.id !== tabId).map((t) => t.id), tabId);
    },
    closeTabsToRight: async (configId, tabId) => {
      const c = conn(configId);
      const idx = c?.sets.tabs.findIndex((t) => t.id === tabId) ?? -1;
      if (c && idx >= 0) await closeTabs(configId, c.sets.tabs.slice(idx + 1).map((t) => t.id), tabId);
    },
    closeTabsToLeft: async (configId, tabId) => {
      const c = conn(configId);
      const idx = c?.sets.tabs.findIndex((t) => t.id === tabId) ?? -1;
      if (c && idx > 0) await closeTabs(configId, c.sets.tabs.slice(0, idx).map((t) => t.id), tabId);
    },
    renameTab: (configId, tabId, title) => {
      const t = title.trim();
      if (!t) return;
      updateTab(configId, tabId, (tab) => (tab.title = t));
      get().scheduleAutosave();
    },
    moveTab: (configId, from, to) => {
      update(configId, (cc) => {
        const tabs = [...cc.sets.tabs];
        const [moved] = tabs.splice(from, 1);
        tabs.splice(to, 0, moved);
        cc.sets = { ...cc.sets, tabs };
      });
      get().scheduleAutosave();
    },
    selectQueryTab: (configId, tabId) => {
      update(configId, (cc) => (cc.sets = { ...cc.sets, selectedTabId: tabId }));
      get().scheduleAutosave();
    },
    nextQueryTab: (direction) => {
      const c = selected();
      if (!c || c.sets.tabs.length === 0) return;
      const idx = Math.max(0, c.sets.tabs.findIndex((t) => t.id === c.sets.selectedTabId));
      const next = (((idx + direction) % c.sets.tabs.length) + c.sets.tabs.length) % c.sets.tabs.length;
      get().selectQueryTab(c.config.id, c.sets.tabs[next].id);
    },
    setQueryText: (configId, tabId, text) => {
      updateTab(configId, tabId, (tab) => (tab.queryText = text));
      get().scheduleAutosave();
    },
    setCursor: (configId, tabId, line, column) =>
      updateTab(configId, tabId, (tab) => {
        tab.cursorLine = line;
        tab.cursorColumn = column;
      }),
    setSelectedResult: (configId, tabId, index) => updateTab(configId, tabId, (tab) => (tab.selectedResultIndex = index)),
    showMessage: (configId, tabId, text, tone) => updateTab(configId, tabId, (tab) => (tab.message = { text, tone })),
    insertSql: (configId, sql) => {
      const c = conn(configId);
      if (!c) return;
      let tab = activeTab(c);
      if (!tab) tab = get().newQueryTab(configId);
      if (!tab) return;
      const combined = appendStatement(tab.queryText, sql);
      get().setQueryText(configId, tab.id, combined);
      getEditor(tab.id)?.setText(combined);
      set({ selectedConnectionId: configId });
    },
    setActiveText: (text) => {
      const c = selected();
      const tab = activeTab(c);
      if (!c || !tab) return;
      get().setQueryText(c.config.id, tab.id, text);
      getEditor(tab.id)?.setText(text);
    },

    // --- execution ---
    execute: async (sqlOverride) => {
      const c = selected();
      const tab = activeTab(c);
      if (!c || !tab) return;
      const editor = getEditor(tab.id);
      const sql = sqlOverride ?? (editor?.getSelection() || editor?.getText() || tab.queryText);
      const result = await get().executeOn(c.config.id, tab.id, sql);
      if (result && sql.trim()) {
        await ipc.addHistory(c.config.id, c.activeDatabase, sql).catch(() => undefined);
        get().bumpHistory();
      }
      await get().saveState();
    },
    executeOn: async (configId, tabId, sql, database) => {
      const c = conn(configId);
      const tab = allTabs(c?.sets ?? emptySets<QueryTab>()).find((t) => t.id === tabId);
      if (!c || !tab) return null;
      if (tab.executing) {
        updateTab(configId, tabId, (t) => (t.statusText = "Query already running (Esc to cancel)"));
        return null;
      }
      if (!sql.trim()) {
        get().showMessage(configId, tabId, "No query to execute.", "warning");
        return null;
      }
      if (!c.connected) {
        get().showMessage(configId, tabId, "Not connected.", "warning");
        return null;
      }
      updateTab(configId, tabId, (t) => {
        t.message = { text: "Executing...", tone: "info" };
        t.result = null;
        t.selectedResultIndex = 0;
        t.statusText = "Executing...";
        t.executionTimeText = "";
        t.executing = true;
        t.executedText = sql;
      });
      try {
        const result = await ipc.executeQuery(configId, tabId, database ?? c.activeDatabase, sql);
        updateTab(configId, tabId, (t) => {
          t.executionTimeText = fmtMs(result.executionMs);
          if (result.cancelled) {
            t.message = { text: "Query cancelled.", tone: "warning" };
            t.rowCountText = "";
            t.statusText = "Query cancelled";
          } else if (result.errorMessage) {
            let msg = result.errorMessage;
            if (result.errorCode != null) msg = `[Error ${result.errorCode}] ${msg}`;
            if (result.errorLine != null && result.errorLine > 0) msg += `\n(Line ${result.errorLine})`;
            t.message = { text: msg, tone: "error" };
            t.rowCountText = "";
            t.statusText = "Query failed";
          } else if (result.data.length > 0 && result.data.some((d) => d.columnNames.length > 0)) {
            // A SELECT with zero rows still has columns: show the headers, not "0 rows affected".
            t.result = result;
            const total = result.data.reduce((n, d) => n + d.rows.length, 0);
            const truncated = result.data.some((d) => d.truncated);
            if (truncated) {
              t.rowCountText = `Showing first ${fmtCount(total)} rows (result truncated)`;
              t.statusText = `Showing first ${fmtCount(total)} rows (result truncated, row limit ${fmtCount(get().appState?.maxRows ?? 0)})`;
            } else {
              t.rowCountText = result.data.length > 1 ? `${fmtCount(total)} rows (${result.data.length} result sets)` : `${fmtCount(total)} rows`;
              t.statusText = `${fmtCount(total)} rows in ${t.executionTimeText}`;
            }
            t.message = null;
          } else {
            t.result = null;
            t.message = { text: `${result.affectedRows} row(s) affected.`, tone: "success" };
            t.rowCountText = `${result.affectedRows} affected`;
            t.statusText = `${result.affectedRows} row(s) affected in ${t.executionTimeText}`;
          }
          if (result.messages.length && !t.result) {
            // Keep server messages visible even without a grid.
            t.result = { ...result, data: [] };
          }
        });
        return result;
      } catch (e) {
        updateTab(configId, tabId, (t) => {
          t.message = { text: errorMessage(e), tone: "error" };
          t.statusText = "Query failed";
        });
        return null;
      } finally {
        updateTab(configId, tabId, (t) => (t.executing = false));
      }
    },
    cancel: (tabId) => {
      const id = tabId ?? activeTab(selected())?.id;
      if (id) void ipc.cancelQuery(id);
    },
    explain: async () => {
      const c = selected();
      const tab = activeTab(c);
      if (!c || !tab) return;
      const editor = getEditor(tab.id);
      const sql = editor?.getSelection() || editor?.getText() || tab.queryText;
      if (!sql.trim()) return;
      await get().executeOn(c.config.id, tab.id, explainSql(c.config.type, sql));
    },
    formatCurrent: async () => {
      const c = selected();
      const tab = activeTab(c);
      if (!c || !tab || !tab.queryText.trim()) return;
      try {
        const formatted = await ipc.formatSql(tab.queryText);
        get().setActiveText(formatted);
      } catch (e) {
        toast.error(errorMessage(e));
      }
    },
    toggleComment: () => {
      const tab = activeTab(selected());
      if (tab) getEditor(tab.id)?.toggleComment();
    },

    // --- settings / layout ---
    setTheme: (theme) => {
      useThemeStore.getState().setTheme(theme);
      set((s) => (s.appState ? { appState: { ...s.appState, theme } } : {}));
      get().scheduleAutosave();
    },
    cycleTheme: () => get().setTheme(useThemeStore.getState().cycle()),
    setMaxRows: (n) => {
      set((s) => (s.appState ? { appState: { ...s.appState, maxRows: Math.max(0, n) } } : {}));
      void get().saveState();
    },
    zoom: (delta) => {
      set((s) => {
        if (!s.appState) return {};
        const size = delta === null ? DEFAULT_FONT : Math.min(MAX_FONT, Math.max(MIN_FONT, s.appState.editorFontSize + delta));
        return { appState: { ...s.appState, editorFontSize: size } };
      });
      get().scheduleAutosave();
    },
    toggleWordWrap: () => {
      set((s) => (s.appState ? { appState: { ...s.appState, editorWordWrap: !s.appState.editorWordWrap } } : {}));
      get().scheduleAutosave();
    },
    toggleConnectionPanel: () => get().setPanelOpen("connection", !get().appState?.isConnectionPanelOpen),
    toggleHistoryPanel: () => get().setPanelOpen("history", !get().appState?.isHistoryPanelOpen),
    setPanelOpen: (panel, open) => {
      set((s) =>
        s.appState ? { appState: { ...s.appState, ...(panel === "connection" ? { isConnectionPanelOpen: open } : { isHistoryPanelOpen: open }) } } : {},
      );
      get().scheduleAutosave();
    },
    setLayout: (patch) => {
      set((s) => (s.appState ? { appState: { ...s.appState, ...patch } } : {}));
      get().scheduleAutosave();
    },

    // --- files ---
    importFile: async (path) => {
      const c = selected();
      if (!c) {
        toast.info("Open a connection first.");
        return;
      }
      try {
        const text = await ipc.readTextFile(path);
        // Reuse the current tab if it's an empty scratch tab, else import into a new one.
        let tab = activeTab(c);
        if (!tab || tab.queryText.trim()) tab = get().newQueryTab(c.config.id);
        if (!tab) return;
        get().renameTab(c.config.id, tab.id, fileStem(path));
        get().setQueryText(c.config.id, tab.id, text);
        getEditor(tab.id)?.setText(text);
        updateTab(c.config.id, tab.id, (t) => (t.statusText = `Imported ${fileName(path)}`));
        await get().saveState();
      } catch (e) {
        const tab = activeTab(c);
        if (tab) get().showMessage(c.config.id, tab.id, `Could not import file: ${errorMessage(e)}`, "error");
      }
    },
    exportFile: async (path) => {
      const c = selected();
      const tab = activeTab(c);
      if (!c || !tab) return;
      try {
        await ipc.writeTextFile(path, getEditor(tab.id)?.getText() ?? tab.queryText);
        updateTab(c.config.id, tab.id, (t) => (t.statusText = `Exported ${fileName(path)}`));
      } catch (e) {
        get().showMessage(c.config.id, tab.id, `Could not export file: ${errorMessage(e)}`, "error");
      }
    },

    // --- persistence ---
    saveState: async () => {
      if (autosaveTimer) {
        clearTimeout(autosaveTimer);
        autosaveTimer = null;
      }
      const s = get();
      if (!s.appState) return;
      for (const c of s.connections) await ipc.saveConnectionState(connectionStateOf(c)).catch(() => undefined);
      const appState: AppState = {
        ...s.appState,
        theme: useThemeStore.getState().theme,
        openConnectionIds: s.connections.map((c) => c.config.id),
        activeConnectionTabId: s.selectedConnectionId,
      };
      try {
        const saved = await ipc.saveAppState(appState);
        set((st) => ({ appState: { ...saved, ...pickLive(st.appState) } }));
      } catch (e) {
        toast.error(`Could not save state: ${errorMessage(e)}`);
      }
    },
    scheduleAutosave: () => {
      if (autosaveTimer) clearTimeout(autosaveTimer);
      autosaveTimer = setTimeout(() => void get().saveState(), 3000);
    },
    bumpHistory: () => set({ historyTick: ++historyTick }),
  };
});

/** Fields the user may have changed while a save was in flight. */
function pickLive(s: AppState | null) {
  if (!s) return {};
  return {
    isConnectionPanelOpen: s.isConnectionPanelOpen,
    isHistoryPanelOpen: s.isHistoryPanelOpen,
    editorFontSize: s.editorFontSize,
    editorWordWrap: s.editorWordWrap,
    maxRows: s.maxRows,
    leftPanelWidth: s.leftPanelWidth,
    editorHeightRatio: s.editorHeightRatio,
    historyPanelWidth: s.historyPanelWidth,
  };
}

export const selectedConnection = (s: WorkspaceState) => s.connections.find((c) => c.config.id === s.selectedConnectionId) ?? null;
export const activeQueryTab = (c: ConnectionTab | null) => c?.sets.tabs.find((t) => t.id === c.sets.selectedTabId) ?? null;
