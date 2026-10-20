import { open, save } from "@tauri-apps/plugin-dialog";
import * as ipc from "./ipc";

const SQL_FILTERS = [
  { name: "SQL files", extensions: ["sql"] },
  { name: "All files", extensions: ["*"] },
];

export async function pickSqlToImport(): Promise<string | null> {
  const r = await open({ title: "Import SQL file", multiple: false, directory: false, filters: SQL_FILTERS });
  return typeof r === "string" ? r : null;
}

export async function pickSqlToExport(suggested: string): Promise<string | null> {
  return (await save({ title: "Export SQL file", defaultPath: suggested, filters: SQL_FILTERS })) ?? null;
}

/** Ask where to save, then write. Returns the chosen path, or null if cancelled. */
export async function saveTextAs(title: string, suggested: string, filters: { name: string; extensions: string[] }[], contents: string): Promise<string | null> {
  const path = await save({ title, defaultPath: suggested, filters });
  if (!path) return null;
  await ipc.writeTextFile(path, contents);
  return path;
}

export async function pickFile(title: string, filters: { name: string; extensions: string[] }[]): Promise<string | null> {
  const r = await open({ title, multiple: false, directory: false, filters });
  return typeof r === "string" ? r : null;
}
