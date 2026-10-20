import { useEffect, useMemo, useRef, useState } from "react";
import { AlertTriangle, Check, ChevronDown, ChevronRight, Copy, FileDown, Square, X } from "lucide-react";
import { copyText } from "../lib/clipboard";
import { saveTextAs } from "../lib/files";
import { errorMessage } from "../lib/format";
import * as ipc from "../lib/ipc";
import * as sql from "../lib/sql";
import type { Cell, ResultSet } from "../lib/types";
import { toast } from "../stores/toast";
import { useUiStore } from "../stores/ui";
import { useWorkspace, type ConnectionTab, type QueryTab } from "../stores/workspace";
import ResultsGrid, { type GridEdit, type GridRow } from "./ResultsGrid";

/** Results: header actions, result-set tabs, truncation banner, filter, grid, messages. */
export default function ResultsPanel({ conn, tab }: { conn: ConnectionTab; tab: QueryTab }) {
  const ws = useWorkspace();
  const askApply = useUiStore((s) => s.askApplyChanges);
  const openMenu = useUiStore((s) => s.openContextMenu);
  const resultsFilterTick = useUiStore((s) => s.resultsFilterTick);
  const setFocusedPane = useUiStore((s) => s.setFocusedPane);
  const result = tab.result;
  const rs: ResultSet | null = result?.data[tab.selectedResultIndex] ?? result?.data[0] ?? null;
  const [filterOpen, setFilterOpen] = useState(false);
  const [filter, setFilter] = useState("");
  const [messagesOpen, setMessagesOpen] = useState(true);
  const filterRef = useRef<HTMLInputElement>(null);
  const [edits, setEdits] = useState<Map<string, Cell>>(new Map()); // "row:col" -> value
  const [version, setVersion] = useState(0); // bumps when edits are reset

  // New results reset edits and the filter.
  useEffect(() => {
    setEdits(new Map());
    setVersion((v) => v + 1);
  }, [result, tab.selectedResultIndex]);

  useEffect(() => {
    if (resultsFilterTick === 0) return;
    if (filterOpen && document.activeElement === filterRef.current) {
      setFilterOpen(false);
      setFilter("");
    } else {
      setFilterOpen(true);
      setTimeout(() => filterRef.current?.select(), 0);
    }
  }, [resultsFilterTick]);

  const queryText = tab.executedText || tab.queryText;
  const tableRef = useMemo(() => sql.parseTableRef(queryText), [queryText]);
  const editable = !!tableRef;

  const rows: GridRow[] = useMemo(() => {
    if (!rs) return [];
    return rs.rows.map((cells, i) => ({ index: i, cells }));
  }, [rs]);

  const current = (row: number, col: number): Cell => {
    const e = edits.get(`${row}:${col}`);
    return e !== undefined ? e : (rs?.rows[row][col] ?? null);
  };

  const filtered = useMemo(() => {
    if (!filter.trim()) return rows;
    const f = filter.trim().toLowerCase();
    const matchNulls = f === "null";
    return rows.filter((r) => r.cells.some((v) => (v === null ? matchNulls : v.toLowerCase().includes(f))));
  }, [rows, filter]);

  const dirtyRows = useMemo(() => {
    const s = new Set<number>();
    for (const k of edits.keys()) s.add(Number(k.split(":")[0]));
    return s;
  }, [edits]);

  const onEdit = ({ row, col, value }: GridEdit) => {
    if (!rs) return;
    setEdits((m) => {
      const n = new Map(m);
      const original = rs.rows[row][col] ?? null;
      const v = value !== null && value.toLowerCase() === "null" ? null : value;
      if (v === original) n.delete(`${row}:${col}`);
      else n.set(`${row}:${col}`, v);
      return n;
    });
  };

  const discard = () => {
    setEdits(new Map());
    setVersion((v) => v + 1);
  };

  const apply = async () => {
    if (!rs || !tableRef || edits.size === 0) return;
    const schema = conn.schema;
    const table = [...(schema?.tables ?? []), ...(schema?.views ?? [])].find(
      (t) => t.name.toLowerCase() === tableRef.table.toLowerCase() && (!tableRef.schema || !t.schema || t.schema.toLowerCase() === tableRef.schema.toLowerCase()),
    );
    const pks = table?.columns.filter((c) => c.isPrimaryKey).map((c) => c.name) ?? [];
    const currentRows = rs.rows.map((r, i) => r.map((_, j) => current(i, j)));
    const script = await ipc.generateUpdateScript({
      tableName: tableRef.table,
      schema: tableRef.schema ?? table?.schema ?? null,
      connectionType: conn.config.type,
      pkColumns: pks,
      columnNames: rs.columnNames,
      columnTypes: rs.columnTypes,
      originalRows: rs.rows,
      currentRows,
      dirtyRows: [...dirtyRows],
      queryText,
    });
    if (script.error) return ws.showMessage(conn.config.id, tab.id, script.error, "warning");
    const answer = await askApply(script.sql, script.expectedStatements);
    if (!answer.execute) return;
    const edited = answer.sql !== script.sql;
    const r = await ws.executeOn(conn.config.id, tab.id, answer.sql).catch((e) => (toast.error(errorMessage(e)), null));
    if (!r) return;
    if (r.cancelled) return ws.showMessage(conn.config.id, tab.id, "Apply cancelled.", "warning");
    if (r.errorMessage) return;
    // executeOn replaced the grid with the script's (empty) result; put the data back with edits committed.
    const committed: ResultSet = { ...rs, rows: currentRows };
    const data = (result?.data ?? []).map((d, i) => (i === tab.selectedResultIndex ? committed : d));
    useWorkspace.setState((s) => ({
      connections: s.connections.map((c) =>
        c.config.id !== conn.config.id
          ? c
          : { ...c, sets: { ...c.sets, tabs: c.sets.tabs.map((t) => (t.id !== tab.id ? t : { ...t, result: { ...(result ?? r), data, messages: r.messages }, executedText: queryText })) } },
      ),
    }));
    if (!edited && r.affectedRows !== script.expectedStatements) {
      const guidance =
        conn.config.type === "sqlServer"
          ? "The transaction was rolled back (row-count check failed)."
          : "The script committed; review the table and run a compensating UPDATE, or ROLLBACK if the session is still open.";
      ws.showMessage(conn.config.id, tab.id, `Warning: expected ${script.expectedStatements} row(s) to change but the server reported ${r.affectedRows}. ${guidance}`, "warning");
      if (conn.config.type === "sqlServer") return; // keep the edits pending for retry / discard
    } else {
      ws.showMessage(conn.config.id, tab.id, edited ? `${r.affectedRows} row(s) affected (script was edited).` : `${r.affectedRows} row(s) updated.`, "success");
    }
    discard();
  };

  const quotedTable = tableRef ? sql.qualified(conn.config.type, tableRef.schema ?? "", tableRef.table) : "<table>";
  const exportCsv = async () => {
    if (!rs) return;
    try {
      const p = await saveTextAs("Export to CSV", "export.csv", [{ name: "CSV Files", extensions: ["csv"] }], sql.csv(rs.columnNames, filtered.map((r) => r.cells)));
      if (p) toast.success(`Exported ${p}`);
    } catch (e) {
      ws.showMessage(conn.config.id, tab.id, `Export failed: ${errorMessage(e)}`, "error");
    }
  };
  const exportJson = async () => {
    if (!rs) return;
    try {
      const p = await saveTextAs("Export to JSON", "export.json", [{ name: "JSON Files", extensions: ["json"] }], sql.json(rs.columnNames, rs.columnTypes, filtered.map((r) => r.cells)));
      if (p) toast.success(`Exported ${p}`);
    } catch (e) {
      ws.showMessage(conn.config.id, tab.id, `Export failed: ${errorMessage(e)}`, "error");
    }
  };
  const copyAll = () => rs && void copyText(sql.tsv(rs.columnNames, filtered.map((r) => r.cells), true)).then((ok) => ok && toast.success("Copied"));

  const contextMenu = (x: number, y: number, selectedRows: GridRow[], cell: { row: number; col: number } | null) => {
    if (!rs) return;
    const selRows = selectedRows.length ? selectedRows : cell ? [filtered.find((r) => r.index === cell.row)!].filter(Boolean) : [];
    openMenu(x, y, [
      { label: "Copy Cell", shortcut: "Ctrl+C", disabled: !cell, onSelect: () => {
          if (cell) void copyText(current(cell.row, cell.col) ?? sql.NULL_LITERAL);
        },
      },
      { label: "Copy Row(s)", shortcut: "Ctrl+Shift+C", disabled: !selRows.length, onSelect: () => void copyText(sql.tsv(rs.columnNames, selRows.map((r) => r.cells), true)) },
      { label: "Copy All Rows", onSelect: copyAll },
      { separator: true, label: "" },
      {
        label: "Copy as INSERT",
        onSelect: () => void copyText(sql.inserts(conn.config.type, quotedTable, rs.columnNames, rs.columnTypes, (selRows.length > 1 ? selRows : filtered).map((r) => r.cells))),
      },
      { separator: true, label: "" },
      { label: "Export to CSV...", onSelect: () => void exportCsv() },
      { label: "Export to JSON...", onSelect: () => void exportJson() },
    ]);
  };

  const hasMessages = (result?.messages.length ?? 0) > 0;
  const tone = tab.message?.tone;
  const toneClass = tone === "error" ? "text-critical" : tone === "warning" ? "text-warning" : tone === "success" ? "text-good" : "text-info";

  return (
    <div className="flex h-full min-h-0 flex-col bg-page" onFocus={() => setFocusedPane("results")}>
      <div className="panel-header gap-3">
        <span>Results</span>
        <span className="font-normal normal-case tracking-normal text-content-muted">{tab.rowCountText}</span>
        <span className="flex-1" />
        {edits.size > 0 && (
          <>
            <button className="btn-chip !border-good/40 !text-good" title="Apply Changes" onClick={() => void apply()}>
              <Check size={11} /> Apply
            </button>
            <button className="btn-chip !border-critical/40 !text-critical" title="Discard Changes" onClick={discard}>
              <X size={11} /> Discard
            </button>
          </>
        )}
        <button className="btn-icon" title="Copy All Rows" disabled={!rs} onClick={copyAll}>
          <Copy size={12} />
        </button>
        <button className="btn-icon" title="Export CSV" disabled={!rs} onClick={() => void exportCsv()}>
          <FileDown size={12} />
        </button>
        {tab.executing && (
          <button className="btn-icon text-critical" title="Cancel Query (Escape)" onClick={() => ws.cancel(tab.id)}>
            <Square size={12} />
          </button>
        )}
      </div>

      {result && result.data.length > 1 && (
        <div className="flex shrink-0 gap-1 border-b border-border-light bg-surface px-1 py-0.5">
          {result.data.map((_, i) => (
            <button key={i} className={`navtab !py-0.5 ${i === tab.selectedResultIndex ? "navtab-active" : ""}`} onClick={() => ws.setSelectedResult(conn.config.id, tab.id, i)}>
              Result {i + 1}
            </button>
          ))}
        </div>
      )}

      {rs?.truncated && (
        <div className="tint-warning flex shrink-0 items-center gap-2 border-b border-warning/40 px-3 py-1 text-[11px]">
          <AlertTriangle size={12} />
          Result truncated at {rs.rows.length.toLocaleString()} rows. Raise the limit under Query → Row limit.
        </div>
      )}

      {filterOpen && (
        <div className="flex shrink-0 items-center gap-1 border-b border-border-light bg-surface px-2 py-1">
          <input
            ref={filterRef}
            autoFocus
            className="h-6 min-w-0 flex-1 bg-transparent text-[11px] text-content outline-none placeholder:text-content-muted"
            placeholder="Filter results... (Ctrl+Shift+R)"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.preventDefault();
                e.stopPropagation();
                setFilter("");
                setFilterOpen(false);
              }
            }}
          />
          <button
            className="btn-icon h-5 w-5"
            title="Clear search"
            onClick={() => {
              setFilter("");
              setFilterOpen(false);
            }}
          >
            <X size={10} />
          </button>
        </div>
      )}

      <div className="min-h-0 flex-1">
        {rs ? (
          <ResultsGrid key={`${version}-${tab.selectedResultIndex}`} columns={rs.columnNames} types={rs.columnTypes} rows={filtered} editable={editable} edits={edits} dirtyRows={dirtyRows} onEdit={onEdit} onContextMenu={contextMenu} />
        ) : (
          <div className="flex h-full items-center justify-center text-xs text-content-muted">{tab.executing ? "Executing…" : "Run a query to see results"}</div>
        )}
      </div>

      {tab.message && (
        <div className={`mono max-h-24 shrink-0 overflow-auto whitespace-pre-wrap border-t border-border-light bg-surface px-3 py-1.5 text-[12px] ${toneClass}`}>{tab.message.text}</div>
      )}
      {hasMessages && (
        <div className="shrink-0 border-t border-border-light bg-surface">
          <button className="flex w-full items-center gap-1 px-2 py-1 text-[11px] font-semibold uppercase tracking-wide text-content-muted hover:text-content" onClick={() => setMessagesOpen((o) => !o)}>
            {messagesOpen ? <ChevronDown size={12} /> : <ChevronRight size={12} />} Messages ({result!.messages.length})
          </button>
          {messagesOpen && <pre className="mono max-h-32 overflow-auto whitespace-pre-wrap px-3 pb-2 text-content-secondary">{result!.messages.join("\n")}</pre>}
        </div>
      )}
    </div>
  );
}
