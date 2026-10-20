import { useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { copyText } from "../lib/clipboard";
import * as sql from "../lib/sql";
import type { Cell } from "../lib/types";
import { toast } from "../stores/toast";

export interface GridRow {
  /** Index in the unfiltered result set. */
  index: number;
  cells: Cell[];
}
export interface GridEdit {
  row: number;
  col: number;
  value: Cell;
}

const ROW_H = 26;
const MIN_W = 60;
const MAX_W = 420;

type Sort = { col: number; asc: boolean } | null;

/** Virtualized, sortable, filterable grid with inline editing (double-click; Enter commits, Esc
 *  cancels; typing NULL writes a null). NULLs render as italic "NULL", sort first. */
export default function ResultsGrid({
  columns,
  types,
  rows,
  editable,
  edits,
  dirtyRows,
  onEdit,
  onContextMenu,
}: {
  columns: string[];
  types: (string | null)[];
  rows: GridRow[];
  editable: boolean;
  edits: Map<string, Cell>;
  dirtyRows: Set<number>;
  onEdit: (e: GridEdit) => void;
  onContextMenu: (x: number, y: number, selectedRows: GridRow[], cell: { row: number; col: number } | null) => void;
}) {
  const parent = useRef<HTMLDivElement>(null);
  const [sort, setSort] = useState<Sort>(null);
  const [selected, setSelected] = useState<Set<number>>(new Set()); // row indices (unfiltered)
  const [anchor, setAnchor] = useState<number | null>(null);
  const [focus, setFocus] = useState<{ row: number; col: number } | null>(null);
  const [editing, setEditing] = useState<{ row: number; col: number; value: string } | null>(null);
  const [widths, setWidths] = useState<number[]>([]);

  const value = (row: GridRow, col: number): Cell => {
    const e = edits.get(`${row.index}:${col}`);
    return e !== undefined ? e : (row.cells[col] ?? null);
  };

  // Pin widths once from headers plus a sample of rows so scrolling doesn't reflow columns.
  useEffect(() => {
    const sample = rows.slice(0, 200);
    setWidths(
      columns.map((c, i) => {
        let max = c.length;
        for (const r of sample) max = Math.max(max, (r.cells[i] ?? "NULL").length);
        return Math.min(MAX_W, Math.max(MIN_W, max * 7.2 + 26));
      }),
    );
  }, [columns, rows.length === 0]);

  const sorted = useMemo(() => {
    if (!sort) return rows;
    const { col, asc } = sort;
    const num = sql.dataKind(types[col] ?? null) === "numeric";
    const key = (r: GridRow) => {
      const v = value(r, col);
      if (v === null) return null;
      if (num || /^-?\d+(\.\d+)?(e[+-]?\d+)?$/i.test(v)) {
        const n = Number(v);
        if (Number.isFinite(n)) return n;
      }
      return v.toLowerCase();
    };
    return [...rows].sort((a, b) => {
      const ka = key(a);
      const kb = key(b);
      let c: number;
      if (ka === null && kb === null) c = 0;
      else if (ka === null) c = -1;
      else if (kb === null) c = 1;
      else if (typeof ka === "number" && typeof kb === "number") c = ka - kb;
      else c = String(ka).localeCompare(String(kb));
      return asc ? c : -c;
    });
  }, [rows, sort, edits, types]);

  const virtualizer = useVirtualizer({ count: sorted.length, getScrollElement: () => parent.current, estimateSize: () => ROW_H, overscan: 20 });
  const totalWidth = 44 + widths.reduce((a, b) => a + b, 0);

  const select = (row: GridRow, e: React.MouseEvent) => {
    setSelected((prev) => {
      if (e.shiftKey && anchor !== null) {
        const a = sorted.findIndex((r) => r.index === anchor);
        const b = sorted.findIndex((r) => r.index === row.index);
        const [lo, hi] = a < b ? [a, b] : [b, a];
        return new Set(sorted.slice(lo, hi + 1).map((r) => r.index));
      }
      if (e.ctrlKey || e.metaKey) {
        const n = new Set(prev);
        if (n.has(row.index)) n.delete(row.index);
        else n.add(row.index);
        return n;
      }
      return new Set([row.index]);
    });
    if (!e.shiftKey) setAnchor(row.index);
  };

  const selectedRows = () => sorted.filter((r) => selected.has(r.index));

  const onKeyDown = async (e: React.KeyboardEvent) => {
    if (editing) return;
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "c") {
      e.preventDefault();
      e.stopPropagation();
      const sel = selectedRows();
      if (e.shiftKey || sel.length > 1) {
        await copyText(sql.tsv(columns, sel.map((r) => r.cells.map((_, i) => value(r, i))), true));
      } else if (focus) {
        const r = rows.find((x) => x.index === focus.row);
        if (r) await copyText(value(r, focus.col) ?? sql.NULL_LITERAL);
      }
      toast.success("Copied");
    } else if ((e.key === "Enter" || e.key === "F2") && focus && editable) {
      e.preventDefault();
      const r = rows.find((x) => x.index === focus.row);
      if (r) setEditing({ row: focus.row, col: focus.col, value: value(r, focus.col) ?? "NULL" });
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const i = sorted.findIndex((r) => r.index === (focus?.row ?? anchor));
      const next = sorted[Math.min(sorted.length - 1, Math.max(0, i + (e.key === "ArrowDown" ? 1 : -1)))];
      if (next) {
        setSelected(new Set([next.index]));
        setAnchor(next.index);
        setFocus({ row: next.index, col: focus?.col ?? 0 });
        virtualizer.scrollToIndex(sorted.indexOf(next));
      }
    } else if ((e.key === "ArrowRight" || e.key === "ArrowLeft") && focus) {
      e.preventDefault();
      setFocus({ row: focus.row, col: Math.min(columns.length - 1, Math.max(0, focus.col + (e.key === "ArrowRight" ? 1 : -1))) });
    }
  };

  const commitEdit = () => {
    if (!editing) return;
    onEdit({ row: editing.row, col: editing.col, value: editing.value });
    setEditing(null);
  };

  return (
    <div
      ref={parent}
      tabIndex={0}
      className="relative h-full overflow-auto outline-none"
      onKeyDown={(e) => void onKeyDown(e)}
      onContextMenu={(e) => {
        e.preventDefault();
        onContextMenu(e.clientX, e.clientY, selectedRows(), focus);
      }}
    >
      <div style={{ width: totalWidth, minWidth: "100%" }}>
        <div className="sticky top-0 z-10 flex bg-surface" style={{ height: ROW_H }}>
          <div className="grid-cell w-11 shrink-0 bg-surface text-right text-content-muted">#</div>
          {columns.map((c, i) => (
            <button
              key={i}
              className="grid-cell flex items-center gap-1 bg-surface text-left text-[11px] font-semibold text-content-muted hover:text-content"
              style={{ width: widths[i] ?? 120, flex: "0 0 auto" }}
              title={types[i] ? `${c} (${types[i]})` : c}
              onClick={() => setSort((s) => (s?.col === i ? (s.asc ? { col: i, asc: false } : null) : { col: i, asc: true }))}
            >
              <span className="truncate">{c}</span>
              {sort?.col === i && <span aria-hidden>{sort.asc ? "▲" : "▼"}</span>}
            </button>
          ))}
        </div>
        <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
          {virtualizer.getVirtualItems().map((vi) => {
            const row = sorted[vi.index];
            const isSel = selected.has(row.index);
            const dirty = dirtyRows.has(row.index);
            return (
              <div
                key={row.index}
                className={`absolute left-0 flex w-full ${isSel ? "bg-accent/15" : vi.index % 2 ? "bg-muted/40" : ""} hover:bg-muted ${dirty ? "grid-cell-dirty" : ""}`}
                style={{ top: vi.start, height: ROW_H }}
                onMouseDown={(e) => {
                  if (e.button === 0) select(row, e);
                }}
              >
                <div className="grid-cell w-11 shrink-0 text-right text-[11px] text-content-muted opacity-70">{row.index + 1}</div>
                {columns.map((_, col) => {
                  const v = value(row, col);
                  const kind = sql.dataKind(types[col] ?? null);
                  const color = v === null ? undefined : kind === "numeric" ? "var(--dt-numeric)" : kind === "date" ? "var(--dt-date)" : kind === "boolean" ? "var(--dt-boolean)" : kind === "binary" ? "var(--dt-binary)" : undefined;
                  const isEditing = editing?.row === row.index && editing.col === col;
                  const isFocus = focus?.row === row.index && focus.col === col;
                  const changed = edits.has(`${row.index}:${col}`);
                  return (
                    <div
                      key={col}
                      className={`grid-cell ${v === null && !isEditing ? "grid-cell-null" : ""} ${isFocus ? "ring-1 ring-inset ring-accent" : ""} ${changed ? "font-semibold" : ""}`}
                      style={{ width: widths[col] ?? 120, flex: "0 0 auto", color, fontFamily: "var(--font-mono)" }}
                      onMouseDown={() => setFocus({ row: row.index, col })}
                      onDoubleClick={() => editable && setEditing({ row: row.index, col, value: v ?? "NULL" })}
                      title={v ?? "NULL"}
                    >
                      {isEditing ? (
                        <input
                          autoFocus
                          className="h-full w-full bg-input px-1 text-[12px] text-content outline-none"
                          value={editing.value}
                          onChange={(e) => setEditing({ ...editing, value: e.target.value })}
                          onFocus={(e) => e.target.select()}
                          onBlur={commitEdit}
                          onKeyDown={(e) => {
                            e.stopPropagation();
                            if (e.key === "Enter") commitEdit();
                            else if (e.key === "Escape") setEditing(null);
                          }}
                        />
                      ) : (
                        (v ?? "NULL")
                      )}
                    </div>
                  );
                })}
              </div>
            );
          })}
        </div>
        {sorted.length === 0 && <div className="px-3 py-4 text-center text-xs text-content-muted">{rows.length === 0 ? "No rows" : "No matches"}</div>}
      </div>
    </div>
  );
}
