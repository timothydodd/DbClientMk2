import { useEffect, useRef, useState } from "react";
import { Plus, X } from "lucide-react";
import { colorForName } from "../lib/colors";
import { useThemeStore } from "../stores/theme";
import { useUiStore } from "../stores/ui";
import { useWorkspace, type ConnectionTab, type QueryTab } from "../stores/workspace";

/** Query sub-tabs for the selected connection: database badge, inline rename, drag to reorder,
 *  close variants from the context menu. */
export default function QueryTabStrip({ conn }: { conn: ConnectionTab }) {
  const ws = useWorkspace();
  const theme = useThemeStore((s) => s.theme);
  const openMenu = useUiStore((s) => s.openContextMenu);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [dragging, setDragging] = useState<string | null>(null);
  const [dropIndex, setDropIndex] = useState<number | null>(null);
  const tabs = conn.sets.tabs;

  const menu = (t: QueryTab, x: number, y: number) =>
    openMenu(x, y, [
      { label: "Close", onSelect: () => void ws.closeQueryTab(conn.config.id, t.id) },
      { label: "Close Others", onSelect: () => void ws.closeOtherTabs(conn.config.id, t.id) },
      { label: "Close to Right", onSelect: () => void ws.closeTabsToRight(conn.config.id, t.id) },
      { label: "Close to Left", onSelect: () => void ws.closeTabsToLeft(conn.config.id, t.id) },
      { separator: true, label: "" },
      { label: "Rename...", onSelect: () => setRenaming(t.id) },
    ]);

  return (
    <div className="flex shrink-0 items-stretch border-b border-border-light bg-surface">
      <div className="flex min-w-0 flex-1 flex-wrap" onDragOver={(e) => e.preventDefault()}>
        {tabs.map((t, i) => {
          const active = t.id === conn.sets.selectedTabId;
          const color = colorForName(t.database, theme);
          return (
            <div
              key={t.id}
              draggable={renaming !== t.id}
              onDragStart={(e) => {
                setDragging(t.id);
                e.dataTransfer.effectAllowed = "move";
              }}
              onDragEnd={() => {
                setDragging(null);
                setDropIndex(null);
              }}
              onDragOver={(e) => {
                e.preventDefault();
                const r = e.currentTarget.getBoundingClientRect();
                setDropIndex(e.clientX < r.left + r.width / 2 ? i : i + 1);
              }}
              onDrop={(e) => {
                e.preventDefault();
                if (dragging && dropIndex !== null) {
                  const from = tabs.findIndex((x) => x.id === dragging);
                  let to = dropIndex;
                  if (from < to) to -= 1;
                  if (from >= 0 && from !== to) ws.moveTab(conn.config.id, from, to);
                  ws.selectQueryTab(conn.config.id, dragging);
                }
                setDragging(null);
                setDropIndex(null);
              }}
              className={`group relative flex h-[30px] cursor-default items-center gap-1.5 px-2.5 text-[11px] ${active ? "text-content" : "text-content-muted hover:text-content"} ${dragging === t.id ? "opacity-40" : ""} ${dropIndex === i && dragging && dragging !== t.id ? "border-l-2 border-accent" : ""}`}
              onClick={() => ws.selectQueryTab(conn.config.id, t.id)}
              onContextMenu={(e) => {
                e.preventDefault();
                menu(t, e.clientX, e.clientY);
              }}
            >
              {t.database && (
                <span className="max-w-[110px] truncate text-[10px]" style={{ color }} title={t.database}>
                  {t.database}
                </span>
              )}
              {renaming === t.id ? (
                <RenameBox
                  value={t.title}
                  onDone={(v) => {
                    if (v) ws.renameTab(conn.config.id, t.id, v);
                    setRenaming(null);
                  }}
                />
              ) : (
                <span className="truncate" onDoubleClick={() => setRenaming(t.id)}>
                  {t.title}
                </span>
              )}
              {t.executing && <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-warning" />}
              <button
                className="rounded opacity-30 hover:bg-muted hover:opacity-100"
                title="Close tab (Ctrl+W)"
                onClick={(e) => {
                  e.stopPropagation();
                  void ws.closeQueryTab(conn.config.id, t.id);
                }}
              >
                <X size={11} />
              </button>
              {active && <span className="absolute inset-x-0 bottom-0 h-0.5" style={{ backgroundColor: color }} />}
              {!active && <span className="absolute right-0 top-2 h-3.5 w-px bg-border-light" />}
            </div>
          );
        })}
      </div>
      <button className="btn-icon m-0.5 h-auto w-8 shrink-0 self-stretch opacity-50 hover:opacity-100" title="New Tab (Ctrl+N)" onClick={() => ws.newQueryTab(conn.config.id)}>
        <Plus size={13} />
      </button>
    </div>
  );
}

function RenameBox({ value, onDone }: { value: string; onDone: (v: string | null) => void }) {
  const [v, setV] = useState(value);
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    ref.current?.focus();
    ref.current?.select();
  }, []);
  return (
    <input
      ref={ref}
      className="h-5 w-24 rounded border border-border bg-input px-1 text-[11px] text-content outline-none focus:border-accent"
      value={v}
      onChange={(e) => setV(e.target.value)}
      onBlur={() => onDone(v.trim() || null)}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") onDone(v.trim() || null);
        else if (e.key === "Escape") onDone(null);
      }}
      onClick={(e) => e.stopPropagation()}
    />
  );
}
