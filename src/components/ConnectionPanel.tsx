import { useEffect, useRef, useState } from "react";
import { AlertTriangle, Database, ListFilter, Pencil, Plug, Plus, RefreshCw, Trash2, X } from "lucide-react";
import { colorForConnection } from "../lib/colors";
import { connectionTitle, defaultConnection, type ConnectionConfig } from "../lib/types";
import { ui, useUiStore } from "../stores/ui";
import { selectedConnection, useWorkspace, type ConnectionTab } from "../stores/workspace";
import { useDacpacActions } from "./DacpacDialogs";
import SchemaTree from "./SchemaTree";
import { EmptyState } from "./ui";

/** Left panel: connection tab strip, schema filter, schema tree (with error / loading / empty
 *  overlays) and the saved-connections overlay. */
export default function ConnectionPanel() {
  const connections = useWorkspace((s) => s.connections);
  const conn = useWorkspace(selectedConnection);
  const saved = useWorkspace((s) => s.appState?.savedConnections ?? []);
  const ws = useWorkspace();
  const openMenu = useUiStore((s) => s.openContextMenu);
  const openDialog = useUiStore((s) => s.openConnectionDialog);
  const [overlay, setOverlay] = useState(false);
  const forced = connections.length === 0;
  const showOverlay = forced || overlay;

  return (
    <div className="flex h-full flex-col bg-surface">
      {!showOverlay && (
        <>
          <ConnectionTabStrip connections={connections} selected={conn} onShowConnections={() => setOverlay(true)} />
          {conn && <FilterBar conn={conn} />}
          <div className="relative min-h-0 flex-1">
            {conn && <TreeArea conn={conn} />}
          </div>
        </>
      )}
      {showOverlay && (
        <div className="flex h-full flex-col">
          <div className="panel-header">
            <Plug size={12} className="text-accent" />
            <span className="flex-1">Connections</span>
            {!forced && (
              <button className="btn-icon" title="Close" onClick={() => setOverlay(false)}>
                <X size={13} />
              </button>
            )}
          </div>
          <div className="p-2">
            <button className="btn-primary w-full" onClick={() => openDialog(defaultConnection())}>
              <Plus size={14} /> New Connection
            </button>
          </div>
          <div className="min-h-0 flex-1 overflow-auto border-t border-border-light p-1">
            {saved.length === 0 && (
              <EmptyState icon={<Database size={28} />} title="No saved connections">
                Add a SQL Server, MySQL or SQLite connection to get started.
              </EmptyState>
            )}
            {saved.map((c) => (
              <SavedConnectionRow
                key={c.id}
                config={c}
                onOpen={() => {
                  setOverlay(false);
                  void ws.openConnection(c.id);
                }}
                onMenu={(x, y) =>
                  openMenu(x, y, [
                    { label: "Connect", icon: <Plug size={14} />, onSelect: () => (setOverlay(false), void ws.openConnection(c.id)) },
                    { label: "Edit...", icon: <Pencil size={14} />, onSelect: () => openDialog(c) },
                    { separator: true, label: "" },
                    {
                      label: "Delete",
                      icon: <Trash2 size={14} />,
                      danger: true,
                      onSelect: () =>
                        void ui.confirm("Delete Connection", `Delete the saved connection "${connectionTitle(c)}"? Its query tabs are deleted too.`, { okText: "Delete", danger: true }).then(
                          (ok) => {
                            if (ok) void ws.deleteSavedConnection(c.id);
                          },
                        ),
                    },
                  ])
                }
              />
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function SavedConnectionRow({ config, onOpen, onMenu }: { config: ConnectionConfig; onOpen: () => void; onMenu: (x: number, y: number) => void }) {
  return (
    <button
      className="flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-left text-sm text-content-secondary hover:bg-muted hover:text-content"
      onClick={onOpen}
      onContextMenu={(e) => {
        e.preventDefault();
        onMenu(e.clientX, e.clientY);
      }}
    >
      <Database size={14} className="shrink-0" style={{ color: colorForConnection(connectionTitle(config)) }} />
      <span className="truncate">{connectionTitle(config)}</span>
    </button>
  );
}

function ConnectionTabStrip({ connections, selected, onShowConnections }: { connections: ConnectionTab[]; selected: ConnectionTab | null; onShowConnections: () => void }) {
  const ws = useWorkspace();
  const openMenu = useUiStore((s) => s.openContextMenu);
  const dacpac = useDacpacActions();
  return (
    <div className="flex shrink-0 items-stretch border-b border-border-light bg-surface">
      <div className="flex min-w-0 flex-1 flex-wrap">
        {connections.map((c) => {
          const title = connectionTitle(c.config);
          const color = colorForConnection(title);
          const active = c.config.id === selected?.config.id;
          return (
            <div
              key={c.config.id}
              className={`group relative flex h-7 max-w-[180px] cursor-default items-center gap-1.5 px-2 text-[11px] ${active ? "text-content" : "text-content-muted hover:text-content"}`}
              onClick={() => ws.selectConnection(c.config.id)}
              onContextMenu={(e) => {
                e.preventDefault();
                openMenu(e.clientX, e.clientY, [
                  ...(c.config.type === "sqlServer" ? [{ label: "Import DACPAC...", onSelect: () => dacpac.importDacpac(c.config.id, null) }] : []),
                  { label: "Close", onSelect: () => void ws.closeConnection(c.config.id) },
                ]);
              }}
              title={title}
            >
              <span className="h-3.5 w-1 shrink-0 rounded" style={{ backgroundColor: color }} />
              <span className="truncate">{title}</span>
              <button
                className="shrink-0 rounded opacity-30 hover:bg-muted hover:opacity-100"
                title="Close connection"
                onClick={(e) => {
                  e.stopPropagation();
                  void ws.closeConnection(c.config.id);
                }}
              >
                <X size={11} />
              </button>
              {active && <span className="absolute inset-x-0 bottom-0 h-0.5" style={{ backgroundColor: color }} />}
            </div>
          );
        })}
      </div>
      <div className="flex shrink-0 items-center gap-0.5 px-1">
        <button className="btn-icon" title="Refresh Schema" onClick={() => selected && void ws.switchDatabase(selected.config.id, selected.activeDatabase, true)}>
          <RefreshCw size={12} />
        </button>
        <button className="btn-icon" title="Connections" onClick={onShowConnections}>
          <Plug size={12} />
        </button>
      </div>
    </div>
  );
}

function FilterBar({ conn }: { conn: ConnectionTab }) {
  const ws = useWorkspace();
  const tick = useUiStore((s) => s.treeFilterTick);
  const openMenu = useUiStore((s) => s.openContextMenu);
  const [text, setText] = useState(conn.treeFilter);
  const input = useRef<HTMLInputElement>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => setText(conn.treeFilter), [conn.config.id, conn.treeFilter]);
  useEffect(() => {
    if (tick > 0) {
      input.current?.focus();
      input.current?.select();
    }
  }, [tick]);

  // Debounce 250 ms before pushing into the store.
  const change = (v: string) => {
    setText(v);
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => ws.setTreeFilter(conn.config.id, v), 250);
  };
  const clear = () => {
    setText("");
    ws.setTreeFilter(conn.config.id, "");
  };

  return (
    <div className="m-1.5 flex items-center gap-1 rounded-full border border-border bg-input px-2 focus-within:border-accent">
      <input
        ref={input}
        className="h-6 min-w-0 flex-1 bg-transparent text-[11px] text-content outline-none placeholder:text-content-muted"
        placeholder="Filter tables, columns..."
        value={text}
        onChange={(e) => change(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            clear();
            (document.querySelector('[data-pane="tree"]') as HTMLElement | null)?.focus();
          }
        }}
      />
      {text && (
        <button className="btn-icon h-5 w-5" title="Clear filter" onClick={clear}>
          <X size={10} />
        </button>
      )}
      <button
        className="btn-icon h-5 w-5"
        title="Filter options"
        onClick={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          openMenu(r.left, r.bottom + 4, [
            { label: "Tables", checked: conn.filterTables, onSelect: () => ws.setFilterScope(conn.config.id, !conn.filterTables, conn.filterColumns) },
            { label: "Columns", checked: conn.filterColumns, onSelect: () => ws.setFilterScope(conn.config.id, conn.filterTables, !conn.filterColumns) },
          ]);
        }}
      >
        <ListFilter size={11} />
      </button>
    </div>
  );
}

function TreeArea({ conn }: { conn: ConnectionTab }) {
  const ws = useWorkspace();
  const dacpac = useDacpacActions();
  if (conn.connectionError) {
    return (
      <div className="flex flex-col items-center gap-3 px-4 py-8 text-center">
        <AlertTriangle size={30} className="text-critical" />
        <div className="text-sm font-semibold text-content">Connection failed</div>
        <div className="max-w-[240px] break-words text-xs text-content-secondary">{conn.connectionError}</div>
        <button className="btn-ghost" onClick={() => void ws.retryConnection(conn.config.id)}>
          Retry
        </button>
      </div>
    );
  }
  if (conn.connecting || (conn.schemaLoading && !conn.schema)) {
    return (
      <div className="flex flex-col gap-[11px] p-2.5">
        {[150, 96, 120, 78, 134, 140, 88, 110, 72, 126, 100].map((w, i) => (
          <div key={i} className="skeleton" style={{ width: w, marginLeft: i === 0 || i === 5 ? 0 : 16 }} />
        ))}
      </div>
    );
  }
  if (conn.connected && conn.databases.length === 0) {
    const isSqlServer = conn.config.type === "sqlServer";
    return (
      <EmptyState icon={<Database size={28} />} title="No databases">
        <p>This server has no user databases yet.</p>
        {isSqlServer && (
          <div className="mt-3 flex flex-col items-center gap-2">
            <button className="btn-ghost" disabled={!dacpac.available} onClick={() => dacpac.importDacpac(conn.config.id, null)}>
              Import DACPAC...
            </button>
            <span>{dacpac.available ? "Creates a database from a .dacpac file using SqlPackage." : `SqlPackage not found. ${dacpac.installHint}`}</span>
          </div>
        )}
      </EmptyState>
    );
  }
  return (
    <div className="relative h-full">
      {conn.schemaLoading && <div className="absolute right-2 top-1 z-10 text-[10px] text-content-muted">Loading…</div>}
      <SchemaTree conn={conn} />
    </div>
  );
}
