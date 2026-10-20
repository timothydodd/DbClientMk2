import { useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openPath, openUrl } from "@tauri-apps/plugin-opener";
import { Braces, FileDown, FileUp, History, Info, Keyboard, Play, Plug, Plus, Square, Table2, X } from "lucide-react";
import * as ipc from "../lib/ipc";
import { pickSqlToExport, pickSqlToImport } from "../lib/files";
import { oneLine } from "../lib/format";
import { defaultConnection, type QueryHistoryEntry } from "../lib/types";
import { useThemeStore } from "../stores/theme";
import { useUiStore, type MenuItemSpec } from "../stores/ui";
import { activeQueryTab, selectedConnection, useWorkspace } from "../stores/workspace";
import { MenuItems } from "./ContextMenu";

export const REPO_URL = "https://github.com/timothydodd/dbclient";

type MenuName = "File" | "Query" | "View" | "Help";

/** File / Query / View / Help menus in the title bar. */
export default function MenuBar() {
  const [open, setOpen] = useState<MenuName | null>(null);
  const [history, setHistory] = useState<QueryHistoryEntry[]>([]);
  const [historyFilter, setHistoryFilter] = useState("");
  const [historyOpen, setHistoryOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const ws = useWorkspace();
  const conn = useWorkspace(selectedConnection);
  const tab = activeQueryTab(conn);
  const theme = useThemeStore((s) => s.theme);
  const uiStore = useUiStore();
  const app = ws.appState;

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(null);
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(null);
    window.addEventListener("mousedown", onDown, true);
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("mousedown", onDown, true);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [open]);

  useEffect(() => {
    if (open !== "Query") setHistoryOpen(false);
  }, [open]);

  useEffect(() => {
    if (!historyOpen) return;
    void ipc.getHistory(null, historyFilter || null).then(setHistory).catch(() => setHistory([]));
  }, [historyOpen, historyFilter, ws.historyTick]);

  const close = () => setOpen(null);
  const importSql = async () => {
    const p = await pickSqlToImport();
    if (p) await ws.importFile(p);
  };
  const exportSql = async () => {
    if (!tab) return;
    const p = await pickSqlToExport(`${tab.title}.sql`);
    if (p) await ws.exportFile(p);
  };

  const rowLimit = (n: number): MenuItemSpec => ({
    label: n === 0 ? "Unlimited" : `${n.toLocaleString()} rows`,
    checked: (app?.maxRows ?? 0) === n,
    onSelect: () => ws.setMaxRows(n),
  });

  const menus: Record<MenuName, MenuItemSpec[]> = {
    File: [
      { label: "New Connection...", icon: <Plug size={14} />, onSelect: () => uiStore.openConnectionDialog(defaultConnection()) },
      { separator: true, label: "" },
      { label: "New Tab", icon: <Plus size={14} />, shortcut: "Ctrl+N", disabled: !conn, onSelect: () => ws.newQueryTab() },
      { label: "Close Tab", icon: <X size={14} />, shortcut: "Ctrl+W", disabled: !tab, onSelect: () => conn && tab && void ws.closeQueryTab(conn.config.id, tab.id) },
      { separator: true, label: "" },
      { label: "Import SQL...", icon: <FileDown size={14} />, shortcut: "Ctrl+O", title: "Copy a .sql file into a new query tab", disabled: !conn, onSelect: () => void importSql() },
      { label: "Export SQL...", icon: <FileUp size={14} />, shortcut: "Ctrl+S", title: "Write the current tab to a .sql file", disabled: !tab, onSelect: () => void exportSql() },
      { label: "Save Workspace", title: "Workspace state autosaves; this forces a save now", onSelect: () => void ws.saveState() },
      { separator: true, label: "" },
      { label: "Exit", shortcut: "Alt+F4", onSelect: () => void getCurrentWindow().close() },
    ],
    Query: [
      { label: "Execute", icon: <Play size={14} />, shortcut: "F5", disabled: !tab, onSelect: () => void ws.execute() },
      { label: "Cancel", icon: <Square size={14} className="text-critical" />, shortcut: "Esc", disabled: !tab?.executing, onSelect: () => ws.cancel() },
      { label: "Explain", icon: <Info size={14} />, shortcut: "Ctrl+E", disabled: !tab, onSelect: () => void ws.explain() },
      { separator: true, label: "" },
      { label: "Format SQL", icon: <Braces size={14} />, shortcut: "Ctrl+Shift+F", disabled: !tab, onSelect: () => void ws.formatCurrent() },
      { separator: true, label: "" },
      { label: app?.maxRows ? `Row limit: ${app.maxRows.toLocaleString()}` : "Row limit: Unlimited", icon: <Table2 size={14} />, disabled: true },
      rowLimit(1_000),
      rowLimit(10_000),
      rowLimit(100_000),
      rowLimit(0),
    ],
    View: [
      { label: "Connection Panel", shortcut: "Ctrl+L", checked: !!app?.isConnectionPanelOpen, onSelect: () => ws.toggleConnectionPanel() },
      { label: "History Panel", shortcut: "Ctrl+H", checked: !!app?.isHistoryPanelOpen, onSelect: () => ws.toggleHistoryPanel() },
      { label: "Focus Schema Filter", shortcut: "Ctrl+Shift+E", onSelect: () => uiStore.focusTreeFilter() },
      { label: "Toggle Results Filter", shortcut: "Ctrl+Shift+R", onSelect: () => uiStore.toggleResultsFilter() },
      { separator: true, label: "" },
      { label: "Word Wrap", shortcut: "Alt+Z", checked: !!app?.editorWordWrap, onSelect: () => ws.toggleWordWrap() },
      { label: "Zoom In", shortcut: "Ctrl+=", onSelect: () => ws.zoom(1) },
      { label: "Zoom Out", shortcut: "Ctrl+-", onSelect: () => ws.zoom(-1) },
      { label: "Reset Zoom", shortcut: "Ctrl+0", onSelect: () => ws.zoom(null) },
      { separator: true, label: "" },
      { label: `Theme: ${theme}`, shortcut: "Ctrl+T", onSelect: () => ws.cycleTheme() },
    ],
    Help: [
      { label: "Keyboard Shortcuts", icon: <Keyboard size={14} />, shortcut: "F1", onSelect: () => uiStore.openAbout("shortcuts") },
      { label: "Open Log Folder", onSelect: () => void ipc.appInfo().then((i) => openPath(i.dataDir)) },
      { label: "GitHub Repository", onSelect: () => void openUrl(REPO_URL) },
      { separator: true, label: "" },
      { label: "About dbclient", icon: <Info size={14} />, onSelect: () => uiStore.openAbout("about") },
    ],
  };

  return (
    <div ref={ref} className="relative flex items-stretch">
      {(Object.keys(menus) as MenuName[]).map((name) => (
        <button
          key={name}
          className={`px-2.5 text-xs transition-colors ${open === name ? "bg-muted text-content" : "text-content-secondary hover:bg-muted hover:text-content"}`}
          onClick={() => setOpen(open === name ? null : name)}
          onMouseEnter={() => open && setOpen(name)}
        >
          {name}
        </button>
      ))}
      {open && (
        <div
          role="menu"
          className="menu !absolute left-0 top-full z-50 mt-0.5 min-w-60"
          style={{ left: ["File", "Query", "View", "Help"].indexOf(open) * 0 }}
        >
          <MenuItems items={menus[open]} onClose={close} />
          {open === "Query" && (
            <>
              <div className="menu-sep" />
              <button className="menu-item" onClick={() => setHistoryOpen((h) => !h)}>
                <span className="flex w-4 items-center justify-center opacity-80">
                  <History size={14} />
                </span>
                <span className="flex-1">History</span>
                <span className="text-[11px] text-content-muted">{historyOpen ? "▲" : "▼"}</span>
              </button>
              {historyOpen && (
                <div className="border-t border-border-light">
                  <input
                    autoFocus
                    className="field m-2 w-[calc(100%-1rem)] text-xs"
                    placeholder="Search history..."
                    value={historyFilter}
                    onChange={(e) => setHistoryFilter(e.target.value)}
                  />
                  <div className="max-h-72 overflow-auto">
                    {history.length === 0 && <div className="px-3 py-2 text-xs text-content-muted">{historyFilter ? "(no matches)" : "(empty)"}</div>}
                    {history.slice(0, 20).map((h, i) => (
                      <button
                        key={i}
                        className="menu-item mono !text-[11px]"
                        title={`${h.connection} / ${h.database}\n${h.executedAt}`}
                        onClick={() => {
                          close();
                          ws.setActiveText(h.query);
                        }}
                      >
                        <span className="w-4" />
                        <span className="flex-1 truncate">{oneLine(h.query, 80)}</span>
                      </button>
                    ))}
                  </div>
                </div>
              )}
            </>
          )}
        </div>
      )}
    </div>
  );
}
