// Dialect helpers for text the UI builds itself (quoting, script templates, exports). The Rust
// core has the tested reference implementations; these mirror them for snappy menus.

import type { Cell, ConnectionType, DbColumn } from "./types";

export const NULL_LITERAL = "NULL";

export function quote(type: ConnectionType, name: string): string {
  switch (type) {
    case "sqlServer":
      return `[${name.replace(/]/g, "]]")}]`;
    case "mySql":
      return `\`${name.replace(/`/g, "``")}\``;
    case "sqlite":
      return `"${name.replace(/"/g, '""')}"`;
  }
}

export function qualified(type: ConnectionType, schema: string, name: string): string {
  return schema ? `${quote(type, schema)}.${quote(type, name)}` : quote(type, name);
}

export const selectAll = (t: ConnectionType, s: string, n: string) => `SELECT * FROM ${qualified(t, s, n)}`;
export const selectTop = (t: ConnectionType, s: string, n: string, count = 100) =>
  t === "sqlServer" ? `SELECT TOP ${count} * FROM ${qualified(t, s, n)}` : `SELECT * FROM ${qualified(t, s, n)} LIMIT ${count}`;
export const selectTopParen = (t: ConnectionType, s: string, n: string, count = 100) =>
  t === "sqlServer" ? `SELECT TOP(${count}) * FROM ${qualified(t, s, n)}` : `SELECT * FROM ${qualified(t, s, n)} LIMIT ${count}`;
export const selectCount = (t: ConnectionType, s: string, n: string) => `SELECT COUNT(*) FROM ${qualified(t, s, n)}`;
export const insertInto = (t: ConnectionType, s: string, n: string) => `INSERT INTO ${qualified(t, s, n)} ()\nVALUES ()`;
export const updateTable = (t: ConnectionType, s: string, n: string) => `UPDATE ${qualified(t, s, n)}\nSET \nWHERE `;
export const deleteFrom = (t: ConnectionType, s: string, n: string) => `DELETE FROM ${qualified(t, s, n)}\nWHERE `;
export const dropTable = (t: ConnectionType, s: string, n: string) => `DROP TABLE ${qualified(t, s, n)}`;
export const dropView = (t: ConnectionType, s: string, n: string) => `DROP VIEW ${qualified(t, s, n)}`;
export const execProc = (t: ConnectionType, s: string, n: string) => (t === "mySql" ? `CALL ${qualified(t, s, n)}()` : `EXEC ${qualified(t, s, n)}`);

export function createTable(t: ConnectionType, schema: string, name: string, columns: DbColumn[]): string {
  const pks = columns.filter((c) => c.isPrimaryKey);
  const lines = columns.map((c, i) => {
    const nullable = c.isNullable && !c.isPrimaryKey;
    let line = `    ${quote(t, c.name)} ${c.dataType}`;
    if (!nullable && !c.isPrimaryKey) line += " NOT NULL";
    if (i < columns.length - 1 || pks.length > 0) line += ",";
    return line;
  });
  if (pks.length) lines.push(`    PRIMARY KEY (${pks.map((c) => quote(t, c.name)).join(", ")})`);
  return `CREATE TABLE ${qualified(t, schema, name)} (\n${lines.join("\n")}\n);`;
}

export function explain(t: ConnectionType, query: string): string {
  if (t === "sqlite") return `EXPLAIN QUERY PLAN ${query}`;
  if (t === "sqlServer") return `SET SHOWPLAN_TEXT ON;\n${query}\nSET SHOWPLAN_TEXT OFF;`;
  return `EXPLAIN ${query}`;
}

export const supportsRenameDatabase = (t: ConnectionType) => t === "sqlServer";
export const supportsDropDatabase = (t: ConnectionType) => t === "sqlServer" || t === "mySql";

/** Append a statement to the editor text with a blank line in between. */
export function appendStatement(existing: string, sql: string): string {
  if (!existing.trim()) return sql;
  const sep = existing.endsWith("\n") ? "\n" : "\n\n";
  return existing.trimEnd() + sep + sql;
}

/** (schema, table) named in the FROM clause, or null. */
export function parseTableRef(query: string): { schema: string | null; table: string } | null {
  const m = /\bFROM\s+([`[\"']?\w+[`\]\"']?(?:\s*\.\s*[`[\"']?\w+[`\]\"']?)?)/i.exec(query);
  if (!m) return null;
  const raw = m[1].replace(/[[\]`"']/g, "");
  if (raw.includes(".")) {
    const parts = raw.split(".");
    return { schema: parts[parts.length - 2].trim(), table: parts[parts.length - 1].trim() };
  }
  return { schema: null, table: raw.trim() };
}

export function isNumericType(t: string | null): boolean {
  if (!t) return false;
  const s = t.toLowerCase();
  return (
    s.includes("int") || s.includes("decimal") || s.includes("numeric") || s.includes("float") || s.includes("double") ||
    s.includes("real") || s.includes("money") || s.includes("number") || s === "bit" || s.includes("serial")
  );
}

export type DataKind = "numeric" | "date" | "boolean" | "binary" | "text";

export function dataKind(t: string | null): DataKind {
  if (!t) return "text";
  const s = t.toLowerCase();
  if (s === "bit" || s.includes("bool")) return "boolean";
  if (isNumericType(s)) return "numeric";
  if (s.includes("date") || s.includes("time")) return "date";
  if (s.includes("binary") || s.includes("blob") || s.includes("image")) return "binary";
  return "text";
}

export function literal(value: Cell, type: string | null): string {
  if (value === null || value.toLowerCase() === "null") return "NULL";
  if (isNumericType(type)) {
    const v = value.trim();
    if (/^true$/i.test(v)) return "1";
    if (/^false$/i.test(v)) return "0";
    if (v !== "" && Number.isFinite(Number(v))) return v;
  }
  return `'${value.replace(/'/g, "''")}'`;
}

// --- exports (NULL: single cell = literal NULL; TSV/CSV = empty; JSON = null) ---

export function tsv(headers: string[], rows: Cell[][], withHeaders: boolean): string {
  const out: string[] = [];
  if (withHeaders) out.push(headers.join("\t"));
  for (const r of rows) out.push(r.map((v) => v ?? "").join("\t"));
  return out.join("\n") + "\n";
}

function csvEscape(v: Cell): string {
  if (v === null) return "";
  return /[",\n]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v;
}

export function csv(headers: string[], rows: Cell[][]): string {
  return [headers.map(csvEscape).join(","), ...rows.map((r) => r.map(csvEscape).join(","))].join("\n") + "\n";
}

export function json(headers: string[], types: (string | null)[], rows: Cell[][]): string {
  const arr = rows.map((r) => {
    const o: Record<string, unknown> = {};
    headers.forEach((h, i) => {
      const v = r[i] ?? null;
      if (v === null) o[h] = null;
      else if (isNumericType(types[i] ?? null) && v.trim() !== "" && Number.isFinite(Number(v))) o[h] = Number(v);
      else o[h] = v;
    });
    return o;
  });
  return JSON.stringify(arr, null, 2);
}

export function inserts(t: ConnectionType, quotedTable: string, headers: string[], types: (string | null)[], rows: Cell[][]): string {
  const cols = headers.map((h) => quote(t, h)).join(", ");
  return rows.map((r) => `INSERT INTO ${quotedTable} (${cols}) VALUES (${headers.map((_, i) => literal(r[i] ?? null, types[i] ?? null)).join(", ")});`).join("\n") + "\n";
}

/** Schema-tree filter: spaces = AND, `|` = OR. */
export function parseFilter(raw: string): string[][] {
  return raw
    .split("|")
    .map((alt) => alt.split(/[ \t]+/).filter(Boolean).map((t) => t.toLowerCase()))
    .filter((alt) => alt.length > 0);
}

export function matchesFilter(name: string, alternatives: string[][]): boolean {
  const lower = name.toLowerCase();
  return alternatives.some((alt) => alt.every((t) => lower.includes(t)));
}
