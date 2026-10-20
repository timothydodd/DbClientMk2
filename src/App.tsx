import { useEffect, useRef } from "react";
import { Database } from "lucide-react";
import AboutDialog from "./components/AboutDialog";
import ApplyChangesDialog from "./components/ApplyChangesDialog";
import ConnectionDialog from "./components/ConnectionDialog";
import ConnectionPanel from "./components/ConnectionPanel";
import ContextMenu from "./components/ContextMenu";
import DacpacDialogs from "./components/DacpacDialogs";
import { ConfirmDialogs, HostKeyDialog, Toasts } from "./components/Dialogs";
import ErrorBoundary from "./components/ErrorBoundary";
import HistoryPanel from "./components/HistoryPanel";
import QueryTabStrip from "./components/QueryTabStrip";
import ResultsPanel from "./components/ResultsPanel";
import Splitter from "./components/Splitter";
import SqlEditor from "./components/SqlEditor";
import StatusBar from "./components/StatusBar";
import TitleBar from "./components/TitleBar";
import { EmptyState } from "./components/ui";
import { useShortcuts } from "./hooks/useShortcuts";
import { activeQueryTab, selectedConnection, useWorkspace } from "./stores/workspace";

export default function App() {
  const init = useWorkspace((s) => s.init);
  const app = useWorkspace((s) => s.appState);
  const conn = useWorkspace(selectedConnection);
  const tab = activeQueryTab(conn);
  const setLayout = useWorkspace((s) => s.setLayout);
  const centerRef = useRef<HTMLDivElement>(null);
  useShortcuts();

  useEffect(() => {
    void init();
  }, [init]);

  // Save on close (the autosave timer may be pending).
  useEffect(() => {
    const onUnload = () => void useWorkspace.getState().saveState();
    window.addEventListener("beforeunload", onUnload);
    return () => window.removeEventListener("beforeunload", onUnload);
  }, []);

  if (!app) return <div className="flex h-full items-center justify-center text-sm text-content-muted">Loading…</div>;

  const leftWidth = app.isConnectionPanelOpen ? Math.max(150, app.leftPanelWidth ?? 250) : 0;
  const historyWidth = app.isHistoryPanelOpen ? Math.max(150, app.historyPanelWidth ?? 280) : 0;
  const ratio = Math.min(0.95, Math.max(0.05, app.editorHeightRatio ?? 0.6));

  return (
    <div className="flex h-full flex-col">
      <TitleBar />
      <div className="flex min-h-0 flex-1">
        {app.isConnectionPanelOpen && (
          <>
            <aside className="shrink-0 border-r border-border-light" style={{ width: leftWidth }}>
              <ErrorBoundary resetKey={conn?.config.id}>
                <ConnectionPanel />
              </ErrorBoundary>
            </aside>
            <Splitter direction="vertical" onDelta={(dx) => setLayout({ leftPanelWidth: Math.max(150, (useWorkspace.getState().appState?.leftPanelWidth ?? 250) + dx) })} />
          </>
        )}
        <main ref={centerRef} className="flex min-w-0 flex-1 flex-col">
          {conn && tab ? (
            <>
              <QueryTabStrip conn={conn} />
              <div className="min-h-0" style={{ flex: `${ratio} 1 0px` }}>
                <ErrorBoundary resetKey={tab.id}>
                  <SqlEditor key={tab.id} connId={conn.config.id} connType={conn.config.type} database={conn.activeDatabase} tab={tab} fontSize={app.editorFontSize} wordWrap={app.editorWordWrap} />
                </ErrorBoundary>
              </div>
              <Splitter
                direction="horizontal"
                onDelta={(dy) => {
                  const h = centerRef.current?.clientHeight ?? 800;
                  const r = useWorkspace.getState().appState?.editorHeightRatio ?? 0.6;
                  setLayout({ editorHeightRatio: Math.min(0.95, Math.max(0.05, r + dy / Math.max(1, h - 31))) });
                }}
              />
              <div className="min-h-0 border-t border-border-light" style={{ flex: `${1 - ratio} 1 0px` }}>
                <ErrorBoundary resetKey={`${tab.id}/results`}>
                  <ResultsPanel conn={conn} tab={tab} />
                </ErrorBoundary>
              </div>
            </>
          ) : (
            <EmptyState icon={<Database size={36} />} title="No connection open">
              Pick a saved connection on the left, or create a new one with File → New Connection.
            </EmptyState>
          )}
        </main>
        {app.isHistoryPanelOpen && (
          <>
            <Splitter direction="vertical" onDelta={(dx) => setLayout({ historyPanelWidth: Math.max(150, (useWorkspace.getState().appState?.historyPanelWidth ?? 280) - dx) })} />
            <aside className="shrink-0 border-l border-border-light" style={{ width: historyWidth }}>
              <ErrorBoundary resetKey="history">
                <HistoryPanel />
              </ErrorBoundary>
            </aside>
          </>
        )}
      </div>
      <StatusBar />
      <ContextMenu />
      <ConnectionDialog />
      <ApplyChangesDialog />
      <DacpacDialogs />
      <HostKeyDialog />
      <AboutDialog />
      <ConfirmDialogs />
      <Toasts />
    </div>
  );
}
