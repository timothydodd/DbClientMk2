import { useEffect, useMemo, useRef, useState } from "react";
import { Menu, type MenuItemOptions, type PredefinedMenuItemOptions, type SubmenuOptions, type CheckMenuItemOptions } from "@tauri-apps/api/menu";
import * as ipc from "../lib/ipc";
import { oneLine } from "../lib/format";
import type { QueryHistoryEntry } from "../lib/types";
import { MENU_NAMES, useAppMenus, type MenuName } from "../hooks/useAppMenus";
import type { MenuItemSpec } from "../stores/ui";
import { useWorkspace } from "../stores/workspace";

type Item = MenuItemOptions | PredefinedMenuItemOptions | SubmenuOptions | CheckMenuItemOptions;

/** Labels that macOS puts elsewhere: Exit is the app menu's Quit, About lives in the app menu. */
const MOVED_TO_APP_MENU = new Set(["Exit", "About dbclient"]);
const RECENT_LIMIT = 20;

/** Drops moved items and any separator left leading, trailing or doubled up. */
function macItems(items: MenuItemSpec[]): MenuItemSpec[] {
  const kept = items.filter((i) => !MOVED_TO_APP_MENU.has(i.label));
  return kept.filter((i, n) => !i.separator || (n > 0 && n < kept.length - 1 && !kept[n - 1].separator));
}

/**
 * Installs the File / Query / View / Help menus as the native macOS menu bar, plus the standard
 * app, Edit and Window menus. Renders nothing.
 *
 * Items carry no accelerators: `useShortcuts` already handles the keys (Ctrl or Cmd), and a native
 * accelerator would fire the action a second time.
 */
export default function NativeMenu() {
  const menus = useAppMenus();
  const historyTick = useWorkspace((s) => s.historyTick);
  const [recent, setRecent] = useState<QueryHistoryEntry[]>([]);

  // Actions read the latest closures through a ref, so the menu only rebuilds when what it shows
  // changes (labels, enabled, checked), not on every render.
  const menusRef = useRef(menus);
  menusRef.current = menus;
  const recentRef = useRef(recent);
  recentRef.current = recent;

  useEffect(() => {
    void ipc.getHistory(null, null).then((h) => setRecent(h.slice(0, RECENT_LIMIT))).catch(() => setRecent([]));
  }, [historyTick]);

  const signature = useMemo(
    () =>
      JSON.stringify([
        MENU_NAMES.map((name) => macItems(menus[name]).map((i) => [i.label, i.separator, i.disabled, i.checked])),
        recent.map((h) => h.query),
      ]),
    [menus, recent],
  );

  const chain = useRef<Promise<void>>(Promise.resolve());
  useEffect(() => {
    chain.current = chain.current.then(async () => {
      try {
        const toNative = (name: MenuName): Item[] =>
          macItems(menusRef.current[name]).map((spec, n): Item => {
            if (spec.separator) return { item: "Separator" };
            const id = `${name}:${n}`;
            const action = () => macItems(menusRef.current[name])[n]?.onSelect?.();
            if (spec.checked !== undefined) return { id, text: spec.label, enabled: !spec.disabled, checked: spec.checked, action };
            return { id, text: spec.label, enabled: !spec.disabled && !!spec.onSelect, action };
          });

        const recentItems: Item[] = recentRef.current.length
          ? recentRef.current.map((h, n) => ({
              id: `recent:${n}`,
              text: oneLine(h.query, 80),
              action: () => {
                const q = recentRef.current[n]?.query;
                if (q !== undefined) useWorkspace.getState().setActiveText(q);
              },
            }))
          : [{ id: "recent:none", text: "(empty)", enabled: false }];

        const menu = await Menu.new({
          items: [
            {
              text: "dbclient",
              items: [
                { id: "about", text: "About dbclient", action: () => menusRef.current.Help.find((i) => i.label === "About dbclient")?.onSelect?.() },
                { item: "Separator" },
                { item: "Services" },
                { item: "Separator" },
                // No "Hide" item: its fixed Cmd+H would shadow the History Panel shortcut.
                { item: "HideOthers" },
                { item: "ShowAll" },
                { item: "Separator" },
                { item: "Quit" },
              ],
            },
            { text: "File", items: toNative("File") },
            {
              text: "Edit",
              items: [{ item: "Undo" }, { item: "Redo" }, { item: "Separator" }, { item: "Cut" }, { item: "Copy" }, { item: "Paste" }, { item: "SelectAll" }],
            },
            { text: "Query", items: [...toNative("Query"), { item: "Separator" }, { text: "History", items: recentItems }] },
            { text: "View", items: toNative("View") },
            { text: "Window", items: [{ item: "Minimize" }, { item: "Maximize" }, { item: "Fullscreen" }, { item: "Separator" }, { item: "BringAllToFront" }] },
            { text: "Help", items: toNative("Help") },
          ],
        });
        const previous = await menu.setAsAppMenu();
        await previous?.close();
      } catch (e) {
        console.error("Failed to build the macOS menu", e);
      }
    });
  }, [signature]);

  return null;
}
