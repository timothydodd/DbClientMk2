import { useEffect, useRef, useState } from "react";
import { History } from "lucide-react";
import * as ipc from "../lib/ipc";
import { oneLine } from "../lib/format";
import type { QueryHistoryEntry } from "../lib/types";
import { MENU_NAMES, useAppMenus, type MenuName } from "../hooks/useAppMenus";
import { useWorkspace } from "../stores/workspace";
import { MenuItems } from "./ContextMenu";

/** File / Query / View / Help menus in the title bar. */
export default function MenuBar() {
  const [open, setOpen] = useState<MenuName | null>(null);
  const [history, setHistory] = useState<QueryHistoryEntry[]>([]);
  const [historyFilter, setHistoryFilter] = useState("");
  const [historyOpen, setHistoryOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const ws = useWorkspace();
  const menus = useAppMenus();

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

  return (
    <div ref={ref} className="relative flex items-stretch">
      {MENU_NAMES.map((name) => (
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
