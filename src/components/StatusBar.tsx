import { Database } from "lucide-react";
import { activeQueryTab, selectedConnection, useWorkspace } from "../stores/workspace";

/** Bottom bar: connection / query status, execution time, caret position. */
export default function StatusBar() {
  const conn = useWorkspace(selectedConnection);
  const tab = activeQueryTab(conn);
  // Connection-level activity or errors win over a stale per-tab result line.
  const status = !conn ? "No connection" : conn.schemaLoading || conn.connectionError || !tab?.statusText ? conn.statusText : tab.statusText;
  return (
    <footer className="flex h-6 shrink-0 items-center gap-4 border-t border-border-light bg-surface px-3 text-[11px] text-content-muted">
      <span className="flex min-w-0 items-center gap-1.5">
        <Database size={11} className="shrink-0 text-accent" />
        <span className="truncate text-content-secondary">{status}</span>
      </span>
      <span className="ml-auto shrink-0">{tab?.executionTimeText ?? ""}</span>
      <span className="shrink-0 text-border">|</span>
      <span className="shrink-0 tabular-nums">
        Ln {tab?.cursorLine ?? 1}, Col {tab?.cursorColumn ?? 1}
      </span>
    </footer>
  );
}
