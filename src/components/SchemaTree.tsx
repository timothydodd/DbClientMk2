import { useEffect, useMemo, useRef, useState } from "react";
import { ChevronDown, ChevronRight, Columns3, Database, Eye, FolderClosed, FolderOpen, Key, Layers, Table2, Zap } from "lucide-react";
import { copyText } from "../lib/clipboard";
import { colorForName } from "../lib/colors";
import * as sql from "../lib/sql";
import type { ConnectionType, DbColumn, DbDatabase } from "../lib/types";
import { useThemeStore } from "../stores/theme";
import { ui, useUiStore, type MenuItemSpec } from "../stores/ui";
import { useWorkspace, type ConnectionTab } from "../stores/workspace";
import { useDacpacActions } from "./DacpacDialogs";

export type NodeType = "database" | "folder" | "schema" | "table" | "view" | "proc" | "column";

export interface TreeNode {
  key: string;
  name: string;
  type: NodeType;
  detail?: string;
  schema?: string;
  columns?: DbColumn[];
  column?: DbColumn;
  children: TreeNode[];
  /** Forced open by the filter. */
  forceOpen?: boolean;
}

function groupBySchema<T extends { schema: string; name: string }>(items: T[], make: (t: T) => TreeNode, parentKey: string, grouping: boolean): TreeNode[] {
  const sorted = [...items].sort((a, b) => a.name.localeCompare(b.name));
  if (!grouping) return sorted.map(make);
  const bySchema = new Map<string, T[]>();
  for (const t of sorted) bySchema.set(t.schema, [...(bySchema.get(t.schema) ?? []), t]);
  return [...bySchema.keys()].sort().map((s) => ({ key: `${parentKey}/schema:${s}`, name: s, type: "schema", children: bySchema.get(s)!.map(make) }));
}

/** Databases as top-level nodes; the active one carries Tables / Views / Stored Procedures. */
export function buildTree(databases: string[], active: string, schema: DbDatabase | null): TreeNode[] {
  return databases.map((db) => {
    const node: TreeNode = { key: `db:${db}`, name: db, type: "database", children: [] };
    if (db !== active || !schema) return node;
    const schemas = new Set([...schema.tables, ...schema.views].map((t) => t.schema).concat(schema.storedProcedures.map((p) => p.schema)).filter(Boolean));
    const grouping = schemas.size > 1;
    const tableNode = (kind: "table" | "view") => (t: DbDatabase["tables"][number]): TreeNode => ({
      key: `${node.key}/${kind}:${t.schema}.${t.name}`,
      name: t.name,
      type: kind,
      schema: t.schema,
      columns: t.columns,
      children: t.columns.map((c) => ({
        key: `${node.key}/${kind}:${t.schema}.${t.name}/col:${c.name}`,
        name: c.name,
        type: "column",
        column: c,
        detail: `${c.dataType}${c.isPrimaryKey ? " PK" : ""}${c.isNullable ? " NULL" : ""}`,
        children: [],
      })),
    });
    if (schema.tables.length) node.children.push({ key: `${node.key}/tables`, name: "Tables", type: "folder", children: groupBySchema(schema.tables, tableNode("table"), `${node.key}/tables`, grouping) });
    if (schema.views.length) node.children.push({ key: `${node.key}/views`, name: "Views", type: "folder", children: groupBySchema(schema.views, tableNode("view"), `${node.key}/views`, grouping) });
    const procNode = (p: DbDatabase["storedProcedures"][number]): TreeNode => ({
      key: `${node.key}/proc:${p.schema}.${p.name}`,
      name: p.name,
      type: "proc",
      schema: p.schema,
      detail: p.parameters.length ? `(${p.parameters.map((x) => x.name).join(", ")})` : undefined,
      children: [],
    });
    if (schema.storedProcedures.length)
      node.children.push({ key: `${node.key}/procs`, name: "Stored Procedures", type: "folder", children: groupBySchema(schema.storedProcedures, procNode, `${node.key}/procs`, grouping) });
    if (schema.functions?.length)
      node.children.push({ key: `${node.key}/funcs`, name: "Functions", type: "folder", children: groupBySchema(schema.functions, procNode, `${node.key}/funcs`, grouping) });
    return node;
  });
}

/** Keep nodes that match (tables/procs by name when the scope includes tables; columns when it includes columns) and their containers. */
export function filterTree(nodes: TreeNode[], alternatives: string[][], tables: boolean, columns: boolean): TreeNode[] {
  const out: TreeNode[] = [];
  for (const n of nodes) {
    const selfMatch =
      n.type === "table" || n.type === "view" || n.type === "proc" ? tables && sql.matchesFilter(n.name, alternatives) : n.type === "column" ? columns && sql.matchesFilter(n.name, alternatives) : false;
    const kids = filterTree(n.children.filter((c) => columns || c.type !== "column"), alternatives, tables, columns);
    if (!selfMatch && kids.length === 0) continue;
    const forceOpen = n.type === "database" || n.type === "folder" || n.type === "schema" ? true : kids.length > 0;
    out.push({ ...n, children: kids, forceOpen });
  }
  return out;
}

const ICONS: Record<NodeType, typeof Table2> = { database: Database, folder: FolderClosed, schema: Layers, table: Table2, view: Eye, proc: Zap, column: Columns3 };

export default function SchemaTree({ conn }: { conn: ConnectionTab }) {
  const theme = useThemeStore((s) => s.theme);
  const ws = useWorkspace();
  const openMenu = useUiStore((s) => s.openContextMenu);
  const setFocusedPane = useUiStore((s) => s.setFocusedPane);
  const dacpac = useDacpacActions();
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  const [selected, setSelected] = useState<string | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  const type: ConnectionType = conn.config.type;

  // The active database starts expanded.
  useEffect(() => {
    if (conn.activeDatabase) setExpanded((s) => new Set([...s, `db:${conn.activeDatabase}`]));
  }, [conn.activeDatabase, conn.schema]);

  const tree = useMemo(() => buildTree(conn.databases, conn.activeDatabase, conn.schema), [conn.databases, conn.activeDatabase, conn.schema]);
  const alternatives = useMemo(() => sql.parseFilter(conn.treeFilter), [conn.treeFilter]);
  const visible = useMemo(() => (alternatives.length ? filterTree(tree, alternatives, conn.filterTables, conn.filterColumns) : tree), [tree, alternatives, conn.filterTables, conn.filterColumns]);

  const insert = (s: string) => ws.insertSql(conn.config.id, s);
  const toggle = (key: string) =>
    setExpanded((s) => {
      const n = new Set(s);
      if (n.has(key)) n.delete(key);
      else n.add(key);
      return n;
    });

  const menuFor = (n: TreeNode): MenuItemSpec[] => {
    const schema = n.schema ?? "";
    const copy: MenuItemSpec = { label: "Copy Name", onSelect: () => void copyText(n.name) };
    switch (n.type) {
      case "database": {
        const items: MenuItemSpec[] = [{ label: "Switch to Database", onSelect: () => void ws.switchDatabase(conn.config.id, n.name) }, { separator: true, label: "" }, copy];
        if (sql.supportsRenameDatabase(type) || sql.supportsDropDatabase(type)) {
          items.push({ separator: true, label: "" });
          if (sql.supportsRenameDatabase(type)) items.push({ label: "Rename Database...", onSelect: () => void renameDatabase(n.name) });
          if (sql.supportsDropDatabase(type)) items.push({ label: "Delete Database...", danger: true, onSelect: () => void deleteDatabase(n.name) });
        }
        if (type === "sqlServer") {
          items.push({ separator: true, label: "" });
          if (dacpac.available) {
            items.push({ label: "Export DACPAC...", onSelect: () => dacpac.exportDacpac(conn.config.id, n.name) });
            items.push({ label: "Import DACPAC...", onSelect: () => dacpac.importDacpac(conn.config.id, n.name) });
          } else {
            items.push({ label: "DACPAC (SqlPackage not found)", disabled: true, title: dacpac.installHint });
          }
        }
        return items;
      }
      case "table":
        return [
          { label: "SELECT * FROM", onSelect: () => insert(sql.selectAll(type, schema, n.name)) },
          { label: type === "sqlServer" ? "SELECT TOP 100" : "SELECT ... LIMIT 100", onSelect: () => insert(sql.selectTop(type, schema, n.name)) },
          { label: "SELECT COUNT(*)", onSelect: () => insert(sql.selectCount(type, schema, n.name)) },
          { separator: true, label: "" },
          { label: "INSERT INTO", onSelect: () => insert(sql.insertInto(type, schema, n.name)) },
          { label: "UPDATE", onSelect: () => insert(sql.updateTable(type, schema, n.name)) },
          { label: "DELETE FROM", onSelect: () => insert(sql.deleteFrom(type, schema, n.name)) },
          { separator: true, label: "" },
          { label: "Script as CREATE TABLE", onSelect: () => insert(sql.createTable(type, schema, n.name, n.columns ?? [])) },
          { label: "Script as DROP TABLE", danger: true, onSelect: () => insert(sql.dropTable(type, schema, n.name)) },
          { separator: true, label: "" },
          copy,
        ];
      case "view":
        return [
          { label: "SELECT * FROM", onSelect: () => insert(sql.selectAll(type, schema, n.name)) },
          { label: type === "sqlServer" ? "SELECT TOP 100" : "SELECT ... LIMIT 100", onSelect: () => insert(sql.selectTop(type, schema, n.name)) },
          { separator: true, label: "" },
          { label: "Script as DROP VIEW", danger: true, onSelect: () => insert(sql.dropView(type, schema, n.name)) },
          { separator: true, label: "" },
          copy,
        ];
      case "proc":
        return [{ label: type === "mySql" ? "CALL" : "EXEC", onSelect: () => insert(sql.execProc(type, schema, n.name)) }, { separator: true, label: "" }, copy];
      case "column":
        return [copy];
      case "folder":
      case "schema":
        return [{ label: "Refresh", onSelect: () => void ws.switchDatabase(conn.config.id, conn.activeDatabase, true) }];
    }
  };

  const renameDatabase = async (database: string) => {
    const newName = await ui.prompt("Rename Database", "New name", database, { okText: "Rename" });
    if (!newName || newName === database) return;
    if (conn.databases.some((d) => d.toLowerCase() === newName.toLowerCase())) return void ui.alert("Rename Database", `A database named "${newName}" already exists.`);
    const ok = await ui.confirm("Rename Database", `Rename "${database}" to "${newName}"?\n\nOther sessions using this database will be disconnected.`, { okText: "Rename" });
    if (!ok) return;
    const error = await ws.renameDatabase(conn.config.id, database, newName);
    if (error) await ui.alert("Rename failed", error);
  };

  const deleteDatabase = async (database: string) => {
    const ok = await ui.confirm("Delete Database", `Permanently delete the database "${database}" and all of its data?\n\nThis cannot be undone.`, { okText: "Delete", danger: true });
    if (!ok) return;
    const typed = await ui.prompt("Confirm Delete", `Type the database name (${database}) to confirm`, "", { okText: "Delete" });
    if (typed === null) return;
    if (typed.toLowerCase() !== database.toLowerCase()) return void ui.alert("Delete Database", "The name did not match. Nothing was deleted.");
    const error = await ws.dropDatabase(conn.config.id, database);
    if (error) await ui.alert("Delete failed", error);
  };

  const onKey = async (e: React.KeyboardEvent) => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "c" && selected) {
      const name = selected.split("/").pop()?.replace(/^(db|table|view|proc|col|schema):/, "").split(".").pop();
      if (name) {
        e.preventDefault();
        await copyText(name);
      }
    }
  };

  const render = (n: TreeNode, depth: number): React.ReactNode => {
    const open = n.forceOpen || expanded.has(n.key);
    const Icon = n.type === "folder" && open ? FolderOpen : ICONS[n.type];
    const hasKids = n.children.length > 0 || (n.type === "database" && n.name === conn.activeDatabase && conn.schemaLoading);
    const color = n.type === "database" ? colorForName(n.name, theme) : undefined;
    const isSel = selected === n.key;
    return (
      <div key={n.key}>
        <div
          className={`flex h-6 cursor-default items-center gap-1 rounded pr-2 text-[12px] ${isSel ? "bg-muted" : "hover:bg-muted"}`}
          style={{ paddingLeft: depth * 14 + 4 }}
          onClick={() => {
            setSelected(n.key);
            if (n.type === "database" && n.name !== conn.activeDatabase) void ws.switchDatabase(conn.config.id, n.name);
            else if (hasKids) toggle(n.key);
          }}
          onDoubleClick={(e) => {
            if (n.type === "table" || n.type === "view") {
              e.stopPropagation();
              insert(sql.selectTopParen(type, n.schema ?? "", n.name));
            }
          }}
          onContextMenu={(e) => {
            e.preventDefault();
            setSelected(n.key);
            const items = menuFor(n);
            if (items.length) openMenu(e.clientX, e.clientY, items);
          }}
        >
          <span
            className="flex w-4 shrink-0 items-center justify-center text-content-muted"
            onClick={(e) => {
              if (hasKids) {
                e.stopPropagation();
                toggle(n.key);
              }
            }}
          >
            {hasKids ? open ? <ChevronDown size={12} /> : <ChevronRight size={12} /> : null}
          </span>
          <Icon size={13} className="shrink-0" style={{ color: color ?? "var(--text-muted)" }} />
          <span className="truncate" style={{ color: color ?? "var(--text-primary)" }}>
            {n.name}
          </span>
          {n.column?.isPrimaryKey && <Key size={10} className="shrink-0 text-warning" />}
          {n.detail && <span className="truncate text-[10px] text-content-muted">{n.detail}</span>}
        </div>
        {open && n.children.map((c) => render(c, depth + 1))}
      </div>
    );
  };

  return (
    <div ref={ref} tabIndex={0} className="h-full overflow-auto p-1 outline-none" onKeyDown={(e) => void onKey(e)} onFocus={() => setFocusedPane("tree")} data-pane="tree">
      {visible.map((n) => render(n, 0))}
      {alternatives.length > 0 && visible.length === 0 && <div className="px-3 py-4 text-center text-xs text-content-muted">No matches</div>}
    </div>
  );
}
