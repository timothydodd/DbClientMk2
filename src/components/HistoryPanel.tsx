import { useEffect, useState } from "react";
import { History, X } from "lucide-react";
import { fmtAgo, oneLine } from "../lib/format";
import * as ipc from "../lib/ipc";
import type { QueryHistoryEntry } from "../lib/types";
import { useThemeStore } from "../stores/theme";
import { colorForName } from "../lib/colors";
import { selectedConnection, useWorkspace } from "../stores/workspace";

/** Right panel: the selected connection's query history, searchable; click loads a query. */
export default function HistoryPanel() {
  const conn = useWorkspace(selectedConnection);
  const tick = useWorkspace((s) => s.historyTick);
  const ws = useWorkspace();
  const theme = useThemeStore((s) => s.theme);
  const [filter, setFilter] = useState("");
  const [entries, setEntries] = useState<QueryHistoryEntry[]>([]);
  const connId = conn?.config.id ?? null;

  useEffect(() => {
    if (!connId) {
      setEntries([]);
      return;
    }
    void ipc.getHistory(connId, filter || null).then(setEntries).catch(() => setEntries([]));
  }, [connId, filter, tick]);

  return (
    <div className="flex h-full flex-col bg-surface">
      <div className="panel-header">
        <History size={12} className="text-accent" />
        <span className="flex-1">History</span>
        <button className="btn-icon" title="Close" onClick={() => ws.setPanelOpen("history", false)}>
          <X size={13} />
        </button>
      </div>
      <div className="p-1.5">
        <input className="field h-7 w-full text-[11px]" placeholder="Search history..." value={filter} onChange={(e) => setFilter(e.target.value)} />
      </div>
      <div className="min-h-0 flex-1 overflow-auto px-1">
        {entries.length === 0 && <div className="px-2 py-3 text-center text-[11px] text-content-muted">{conn ? "No history" : "No connection selected"}</div>}
        {entries.map((e, i) => (
          <button key={i} className="flex w-full flex-col gap-0.5 rounded-md px-2 py-1.5 text-left hover:bg-muted" title={e.query} onClick={() => ws.setActiveText(e.query)}>
            <span className="mono line-clamp-2 w-full break-all text-[11px] text-content">{oneLine(e.query, 120)}</span>
            <span className="flex gap-2 text-[9px] text-content-muted">
              <span style={{ color: colorForName(e.database, theme) }}>{e.database}</span>
              <span>{fmtAgo(e.executedAt)}</span>
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}
