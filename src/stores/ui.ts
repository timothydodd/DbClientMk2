// Overlay state: dialogs (promise-based confirm / prompt), context menus, the connection editor,
// SSH host-key prompts, DACPAC dialogs, About.

import { create } from "zustand";
import type { ReactNode } from "react";
import type { ConnectionConfig, HostKeyEvent } from "../lib/types";

export interface ConfirmRequest {
  id: number;
  title: string;
  message: ReactNode;
  okText: string;
  danger?: boolean;
  /** Just an OK button (an alert). */
  alert?: boolean;
  resolve: (ok: boolean) => void;
}

export interface PromptRequest {
  id: number;
  title: string;
  label: ReactNode;
  initial: string;
  okText: string;
  placeholder?: string;
  resolve: (value: string | null) => void;
}

export interface MenuItemSpec {
  label: string;
  onSelect?: () => void;
  disabled?: boolean;
  danger?: boolean;
  title?: string;
  icon?: ReactNode;
  separator?: boolean;
  checked?: boolean;
  shortcut?: string;
}

export interface ContextMenuState {
  x: number;
  y: number;
  items: MenuItemSpec[];
}

export type DacpacDialog =
  | { kind: "export"; connectionId: string; database: string }
  | { kind: "import"; connectionId: string; database: string | null }
  | null;

export interface SqlPackageRun {
  jobId: string;
  title: string;
  message: string;
  args: string[];
  /** Called with success once the process exits. */
  onDone?: (ok: boolean) => void;
}

interface UiState {
  confirms: ConfirmRequest[];
  prompts: PromptRequest[];
  contextMenu: ContextMenuState | null;
  /** Connection dialog: null = closed; a config = editing (empty id = new). */
  connectionDialog: ConnectionConfig | null;
  hostKeys: HostKeyEvent[];
  dacpac: DacpacDialog;
  sqlpackageRun: SqlPackageRun | null;
  about: "about" | "shortcuts" | null;
  applyChanges: { sql: string; expected: number; resolve: (r: { execute: boolean; sql: string }) => void } | null;
  /** Which pane has keyboard focus, so Ctrl+F can go to the right search box. */
  focusedPane: "editor" | "results" | "tree" | null;
  /** Bumped to ask the schema panel / results panel to focus or toggle their filter box. */
  treeFilterTick: number;
  resultsFilterTick: number;
  setFocusedPane: (p: "editor" | "results" | "tree" | null) => void;
  focusTreeFilter: () => void;
  toggleResultsFilter: () => void;
  confirm: (title: string, message: ReactNode, opts?: { okText?: string; danger?: boolean }) => Promise<boolean>;
  alert: (title: string, message: ReactNode) => Promise<void>;
  prompt: (title: string, label: ReactNode, initial?: string, opts?: { okText?: string; placeholder?: string }) => Promise<string | null>;
  resolveConfirm: (id: number, ok: boolean) => void;
  resolvePrompt: (id: number, value: string | null) => void;
  openContextMenu: (x: number, y: number, items: MenuItemSpec[]) => void;
  closeContextMenu: () => void;
  openConnectionDialog: (config: ConnectionConfig) => void;
  closeConnectionDialog: () => void;
  pushHostKey: (e: HostKeyEvent) => void;
  popHostKey: (requestId: string) => void;
  openDacpac: (d: DacpacDialog) => void;
  openSqlPackageRun: (r: SqlPackageRun) => void;
  closeSqlPackageRun: () => void;
  openAbout: (page: "about" | "shortcuts") => void;
  closeAbout: () => void;
  askApplyChanges: (sql: string, expected: number) => Promise<{ execute: boolean; sql: string }>;
  closeApplyChanges: () => void;
}

let nextId = 1;

export const useUiStore = create<UiState>((set, get) => ({
  confirms: [],
  prompts: [],
  contextMenu: null,
  connectionDialog: null,
  hostKeys: [],
  dacpac: null,
  sqlpackageRun: null,
  about: null,
  applyChanges: null,
  focusedPane: null,
  treeFilterTick: 0,
  resultsFilterTick: 0,
  setFocusedPane: (focusedPane) => set({ focusedPane }),
  focusTreeFilter: () => set((s) => ({ treeFilterTick: s.treeFilterTick + 1 })),
  toggleResultsFilter: () => set((s) => ({ resultsFilterTick: s.resultsFilterTick + 1 })),

  confirm: (title, message, opts) =>
    new Promise<boolean>((resolve) => {
      const id = nextId++;
      set((s) => ({ confirms: [...s.confirms, { id, title, message, okText: opts?.okText ?? "OK", danger: opts?.danger, resolve }] }));
    }),
  alert: (title, message) =>
    new Promise<void>((resolve) => {
      const id = nextId++;
      set((s) => ({ confirms: [...s.confirms, { id, title, message, okText: "OK", alert: true, resolve: () => resolve() }] }));
    }),
  prompt: (title, label, initial = "", opts) =>
    new Promise<string | null>((resolve) => {
      const id = nextId++;
      set((s) => ({ prompts: [...s.prompts, { id, title, label, initial, okText: opts?.okText ?? "OK", placeholder: opts?.placeholder, resolve }] }));
    }),
  resolveConfirm: (id, ok) => {
    const c = get().confirms.find((x) => x.id === id);
    set((s) => ({ confirms: s.confirms.filter((x) => x.id !== id) }));
    c?.resolve(ok);
  },
  resolvePrompt: (id, value) => {
    const p = get().prompts.find((x) => x.id === id);
    set((s) => ({ prompts: s.prompts.filter((x) => x.id !== id) }));
    p?.resolve(value);
  },
  openContextMenu: (x, y, items) => set({ contextMenu: { x, y, items } }),
  closeContextMenu: () => set({ contextMenu: null }),
  openConnectionDialog: (config) => set({ connectionDialog: config }),
  closeConnectionDialog: () => set({ connectionDialog: null }),
  pushHostKey: (e) => set((s) => ({ hostKeys: [...s.hostKeys, e] })),
  popHostKey: (requestId) => set((s) => ({ hostKeys: s.hostKeys.filter((h) => h.requestId !== requestId) })),
  openDacpac: (d) => set({ dacpac: d }),
  openSqlPackageRun: (r) => set({ sqlpackageRun: r }),
  closeSqlPackageRun: () => set({ sqlpackageRun: null }),
  openAbout: (page) => set({ about: page }),
  closeAbout: () => set({ about: null }),
  askApplyChanges: (sql, expected) => new Promise((resolve) => set({ applyChanges: { sql, expected, resolve } })),
  closeApplyChanges: () => set({ applyChanges: null }),
}));

export const ui = {
  confirm: (title: string, message: ReactNode, opts?: { okText?: string; danger?: boolean }) => useUiStore.getState().confirm(title, message, opts),
  alert: (title: string, message: ReactNode) => useUiStore.getState().alert(title, message),
  prompt: (title: string, label: ReactNode, initial?: string, opts?: { okText?: string; placeholder?: string }) =>
    useUiStore.getState().prompt(title, label, initial, opts),
};
