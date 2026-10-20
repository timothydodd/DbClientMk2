// Each connection keeps a separate set of query tabs per database. The active database's tabs
// are `tabs`; the others are stashed under their database name. The "" key holds tabs created
// before any database was active; they migrate into the first database that becomes active.

import type { TabState } from "./types";

export interface HasDatabase {
  id: string;
  title: string;
  queryText: string;
  database: string;
}

export interface TabSets<T extends HasDatabase> {
  tabs: T[];
  selectedTabId: string | null;
  stashed: Record<string, T[]>;
  stashedSelected: Record<string, string | null>;
  tabsActiveDb: string;
}

export function emptySets<T extends HasDatabase>(): TabSets<T> {
  return { tabs: [], selectedTabId: null, stashed: {}, stashedSelected: {}, tabsActiveDb: "" };
}

/** Swap the visible tab set to `database`'s (returns a new sets object; tabs keep identity). */
export function activateDatabaseTabs<T extends HasDatabase>(s: TabSets<T>, database: string): TabSets<T> {
  if (database === s.tabsActiveDb) return s;
  const stashed: Record<string, T[]> = { ...s.stashed };
  const stashedSelected: Record<string, string | null> = { ...s.stashedSelected };
  const prev = s.tabsActiveDb;
  const carryOverKey = prev || database;

  if (s.tabs.length > 0) {
    stashed[carryOverKey] = [...(stashed[carryOverKey] ?? []), ...s.tabs];
    stashedSelected[carryOverKey] = s.selectedTabId;
  }

  // Orphaned (unassigned) tabs migrate into the target database.
  if (database && carryOverKey !== "" && stashed[""]) {
    stashed[database] = [...(stashed[database] ?? []), ...stashed[""]];
    delete stashed[""];
    if (!(database in stashedSelected) && "" in stashedSelected) stashedSelected[database] = stashedSelected[""];
    delete stashedSelected[""];
  }

  const tabs = (stashed[database] ?? []).map((t) => ({ ...t, database }));
  delete stashed[database];
  const selId = stashedSelected[database] ?? null;
  delete stashedSelected[database];
  const selectedTabId = (selId && tabs.find((t) => t.id === selId)?.id) ?? tabs[0]?.id ?? null;
  return { tabs, selectedTabId, stashed, stashedSelected, tabsActiveDb: database };
}

/** Add restored tabs for `database` to the stash (used before the first activation). */
export function restoreTabsForDatabase<T extends HasDatabase>(s: TabSets<T>, database: string, tabs: T[], activeTabId: string | null): TabSets<T> {
  if (tabs.length === 0) return s;
  const key = database ?? "";
  const stashed = { ...s.stashed, [key]: [...(s.stashed[key] ?? []), ...tabs.map((t) => ({ ...t, database: key }))] };
  const stashedSelected = { ...s.stashedSelected };
  if (activeTabId) stashedSelected[key] = activeTabId;
  return { ...s, stashed, stashedSelected };
}

export function collectAllTabStates<T extends HasDatabase>(s: TabSets<T>): TabState[] {
  const out: TabState[] = s.tabs.map((t, i) => ({ id: t.id, title: t.title, queryText: t.queryText, database: s.tabsActiveDb, order: i }));
  for (const [db, tabs] of Object.entries(s.stashed)) {
    tabs.forEach((t, i) => out.push({ id: t.id, title: t.title, queryText: t.queryText, database: db, order: i }));
  }
  return out;
}

export function activeTabIdsByDatabase<T extends HasDatabase>(s: TabSets<T>): Record<string, string | null> {
  const out: Record<string, string | null> = { ...s.stashedSelected };
  if (s.tabsActiveDb) out[s.tabsActiveDb] = s.selectedTabId;
  return out;
}

export function allTabs<T extends HasDatabase>(s: TabSets<T>): T[] {
  return [...s.tabs, ...Object.values(s.stashed).flat()];
}

/** Rename a database's tab set (after a server-side rename). */
export function renameTabSet<T extends HasDatabase>(s: TabSets<T>, oldName: string, newName: string): TabSets<T> {
  const stashed = { ...s.stashed };
  const stashedSelected = { ...s.stashedSelected };
  if (stashed[oldName]) {
    stashed[newName] = stashed[oldName].map((t) => ({ ...t, database: newName }));
    delete stashed[oldName];
  }
  if (oldName in stashedSelected) {
    stashedSelected[newName] = stashedSelected[oldName];
    delete stashedSelected[oldName];
  }
  if (s.tabsActiveDb === oldName) {
    return { ...s, stashed, stashedSelected, tabsActiveDb: newName, tabs: s.tabs.map((t) => ({ ...t, database: newName })) };
  }
  return { ...s, stashed, stashedSelected };
}

/** Drop a database's stashed tabs (after a server-side drop). */
export function dropTabSet<T extends HasDatabase>(s: TabSets<T>, name: string): TabSets<T> {
  const stashed = { ...s.stashed };
  const stashedSelected = { ...s.stashedSelected };
  delete stashed[name];
  delete stashedSelected[name];
  return { ...s, stashed, stashedSelected };
}
