import { useEffect } from "react";
import { pickSqlToExport, pickSqlToImport } from "../lib/files";
import { useUiStore } from "../stores/ui";
import { activeQueryTab, getEditor, selectedConnection, useWorkspace } from "../stores/workspace";
import { overlaysOpen } from "../components/ui";

/** Window-level shortcuts. The editor handles its own keys first and marks them handled. */
export function useShortcuts() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return;
      const ws = useWorkspace.getState();
      const uiState = useUiStore.getState();
      const conn = selectedConnection(ws);
      const tab = activeQueryTab(conn);
      const ctrl = e.ctrlKey || e.metaKey;
      const key = e.key.toLowerCase();
      const handled = () => {
        e.preventDefault();
        e.stopPropagation();
      };

      if (e.key === "F5") {
        handled();
        void ws.execute();
        return;
      }
      if (e.key === "F1") {
        handled();
        uiState.openAbout("shortcuts");
        return;
      }
      if (e.key === "Escape" && !overlaysOpen() && !uiState.contextMenu && tab?.executing) {
        handled();
        ws.cancel();
        return;
      }
      if (e.altKey && !ctrl && key === "z") {
        handled();
        ws.toggleWordWrap();
        return;
      }
      if (!ctrl) return;

      if (e.key === "Enter") {
        handled();
        void ws.execute();
        return;
      }
      if (e.key === "Tab") {
        handled();
        ws.nextQueryTab(e.shiftKey ? -1 : 1);
        return;
      }
      if (e.shiftKey) {
        if (key === "f") {
          handled();
          void ws.formatCurrent();
        } else if (key === "e") {
          handled();
          uiState.focusTreeFilter();
        } else if (key === "r") {
          handled();
          uiState.toggleResultsFilter();
        }
        return;
      }
      switch (key) {
        case "n":
          handled();
          ws.newQueryTab();
          return;
        case "w":
          handled();
          if (conn && tab) void ws.closeQueryTab(conn.config.id, tab.id);
          return;
        case "o":
          handled();
          void pickSqlToImport().then((p) => {
            if (p) void ws.importFile(p);
          });
          return;
        case "s":
          handled();
          if (tab)
            void pickSqlToExport(`${tab.title}.sql`).then((p) => {
              if (p) void ws.exportFile(p);
            });
          return;
        case "e":
          handled();
          void ws.explain();
          return;
        case "/":
          handled();
          ws.toggleComment();
          return;
        case "=":
        case "+":
          handled();
          ws.zoom(1);
          return;
        case "-":
          handled();
          ws.zoom(-1);
          return;
        case "0":
          handled();
          ws.zoom(null);
          return;
        case "l":
          handled();
          ws.toggleConnectionPanel();
          return;
        case "h":
          handled();
          ws.toggleHistoryPanel();
          return;
        case "t":
          handled();
          ws.cycleTheme();
          return;
        case "f":
          // Find in the focused pane: the editor's search is its own key binding.
          if (uiState.focusedPane === "results") {
            handled();
            uiState.toggleResultsFilter();
          } else if (uiState.focusedPane === "tree") {
            handled();
            uiState.focusTreeFilter();
          } else if (tab) {
            handled();
            getEditor(tab.id)?.openSearch();
          }
          return;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}
