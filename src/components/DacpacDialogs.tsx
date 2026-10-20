import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { save } from "@tauri-apps/plugin-dialog";
import { pickFile } from "../lib/files";
import { errorMessage, fileName, fileStem } from "../lib/format";
import * as ipc from "../lib/ipc";
import type { SqlPackageLine } from "../lib/types";
import { toast } from "../stores/toast";
import { ui, useUiStore } from "../stores/ui";
import { useWorkspace } from "../stores/workspace";
import { WarningNote } from "./Dialogs";
import { Checkbox, Field, Modal } from "./ui";

export const INSTALL_HINT = "Install with: dotnet tool install -g microsoft.sqlpackage";
const DACPAC_FILTER = [{ name: "DACPAC files", extensions: ["dacpac"] }];

let sqlpackageAvailable: boolean | null = null;
const listeners = new Set<() => void>();
function setAvailable(v: boolean) {
  sqlpackageAvailable = v;
  listeners.forEach((l) => l());
}
/** Looked up once; the About dialog's recheck can refresh it. */
export async function checkSqlPackage(recheck = false) {
  try {
    setAvailable(!!(await ipc.sqlpackagePath(recheck)));
  } catch {
    setAvailable(false);
  }
}

export function useDacpacActions() {
  const openDacpac = useUiStore((s) => s.openDacpac);
  const [available, setAv] = useState(sqlpackageAvailable ?? false);
  useEffect(() => {
    const l = () => setAv(sqlpackageAvailable ?? false);
    listeners.add(l);
    if (sqlpackageAvailable === null) void checkSqlPackage();
    return () => void listeners.delete(l);
  }, []);
  return {
    available,
    installHint: INSTALL_HINT,
    exportDacpac: (connectionId: string, database: string) => openDacpac({ kind: "export", connectionId, database }),
    importDacpac: (connectionId: string, database: string | null) => {
      if (!available) {
        void ui.alert("SqlPackage not found", `DACPAC import needs the SqlPackage command-line tool.\n\n${INSTALL_HINT}`);
        return;
      }
      openDacpac({ kind: "import", connectionId, database });
    },
  };
}

export default function DacpacDialogs() {
  const d = useUiStore((s) => s.dacpac);
  const run = useUiStore((s) => s.sqlpackageRun);
  return (
    <>
      {d?.kind === "export" && <ExportDialog connectionId={d.connectionId} database={d.database} />}
      {d?.kind === "import" && <ImportDialog connectionId={d.connectionId} database={d.database} />}
      {run && <RunDialog />}
    </>
  );
}

function ExportDialog({ connectionId, database }: { connectionId: string; database: string }) {
  const close = () => useUiStore.getState().openDacpac(null);
  const openRun = useUiStore((s) => s.openSqlPackageRun);
  const [includeData, setIncludeData] = useState(false);
  const start = async () => {
    const path = await save({ title: `Export ${database} as DACPAC`, defaultPath: `${database}.dacpac`, filters: DACPAC_FILTER });
    if (!path) return;
    close();
    try {
      const cs = await ipc.externalConnectionString(connectionId, database);
      openRun({
        jobId: crypto.randomUUID(),
        title: "Export DACPAC",
        message: `Extracting ${database} to ${fileName(path)}`,
        args: ["/Action:Extract", `/SourceConnectionString:${cs}`, `/TargetFile:${path}`, `/p:ExtractAllTableData=${includeData ? "True" : "False"}`, "/p:VerifyExtraction=False"],
        onDone: (ok) => toast[ok ? "success" : "error"](ok ? `Exported ${fileName(path)}` : "DACPAC export failed"),
      });
    } catch (e) {
      toast.error(errorMessage(e));
    }
  };
  return (
    <Modal title="Export DACPAC" onClose={close}>
      <div className="flex flex-col gap-3 text-sm">
        <p className="text-content-secondary">Export database "{database}" to a .dacpac file.</p>
        <Checkbox checked={includeData} onChange={setIncludeData} label="Include table data (schema + data)" />
        <div className="flex justify-end gap-2">
          <button className="btn-ghost" onClick={close}>
            Cancel
          </button>
          <button className="btn-primary" onClick={() => void start()}>
            Export...
          </button>
        </div>
      </div>
    </Modal>
  );
}

function ImportDialog({ connectionId, database }: { connectionId: string; database: string | null }) {
  const close = () => useUiStore.getState().openDacpac(null);
  const openRun = useUiStore((s) => s.openSqlPackageRun);
  const ws = useWorkspace();
  const conn = useWorkspace((s) => s.connections.find((c) => c.config.id === connectionId));
  const [file, setFile] = useState("");
  const [target, setTarget] = useState(database ?? "");
  const [block, setBlock] = useState(true);
  const [scriptOnly, setScriptOnly] = useState(false);
  const [exists, setExists] = useState(false);
  useEffect(() => {
    if (!file) return;
    void ipc.fileExists(file).then(setExists).catch(() => setExists(false));
  }, [file]);
  const databases = conn?.databases ?? [];
  const targetExists = !!target && databases.some((d) => d.toLowerCase() === target.toLowerCase());
  const valid = !!file && exists && !!target.trim();

  const browse = async () => {
    const p = await pickFile("Select DACPAC", [...DACPAC_FILTER, { name: "All files", extensions: ["*"] }]);
    if (!p) return;
    setFile(p);
    if (!target.trim()) setTarget(fileStem(p));
  };

  const start = async () => {
    const t = target.trim();
    try {
      const cs = await ipc.externalConnectionString(connectionId, t);
      if (scriptOnly) {
        const out = await save({ title: "Save deployment script", defaultPath: `${t}.deploy.sql`, filters: [{ name: "SQL files", extensions: ["sql"] }] });
        if (!out) return;
        close();
        openRun({
          jobId: crypto.randomUUID(),
          title: "Generate deployment script",
          message: `Scripting ${fileName(file)} against ${t}`,
          args: ["/Action:Script", `/SourceFile:${file}`, `/TargetConnectionString:${cs}`, `/OutputPath:${out}`, `/p:BlockOnPossibleDataLoss=${block ? "True" : "False"}`],
          onDone: (ok) => {
            if (!ok) return;
            void ipc.readTextFile(out).then((text) => {
              const tab = ws.newQueryTab(connectionId, fileStem(out), text);
              if (tab) ws.selectQueryTab(connectionId, tab.id);
            });
          },
        });
        return;
      }
      const message = targetExists
        ? `Publish ${fileName(file)} to the EXISTING database "${t}"?\n\nObjects will be altered or dropped to match the DACPAC.`
        : `Publish ${fileName(file)} as a new database "${t}"?`;
      if (!(await ui.confirm("Import DACPAC", message, { okText: "Publish", danger: targetExists }))) return;
      close();
      openRun({
        jobId: crypto.randomUUID(),
        title: "Import DACPAC",
        message: `Publishing ${fileName(file)} to ${t}`,
        args: ["/Action:Publish", `/SourceFile:${file}`, `/TargetConnectionString:${cs}`, `/p:BlockOnPossibleDataLoss=${block ? "True" : "False"}`],
        onDone: async (ok) => {
          if (!ok) return toast.error("DACPAC import failed");
          toast.success(`Imported ${t}`);
          await ws.reloadDatabases(connectionId);
          const c = useWorkspace.getState().connections.find((x) => x.config.id === connectionId);
          if (c && (!c.activeDatabase || c.activeDatabase.toLowerCase() === t.toLowerCase())) await ws.switchDatabase(connectionId, t, true);
        },
      });
    } catch (e) {
      toast.error(errorMessage(e));
    }
  };

  return (
    <Modal title="Import DACPAC" onClose={close} wide>
      <div className="flex flex-col gap-3 text-sm">
        <Field label="DACPAC file">
          <div className="flex gap-2">
            <input className="field w-full" value={file} onChange={(e) => setFile(e.target.value)} placeholder="C:\path\to\database.dacpac" />
            <button className="btn-ghost shrink-0" onClick={() => void browse()}>
              Browse...
            </button>
          </div>
        </Field>
        <Field label="Target database" hint="Created when it does not exist.">
          <input className="field w-full" value={target} onChange={(e) => setTarget(e.target.value)} list="dacpac-dbs" />
          <datalist id="dacpac-dbs">
            {databases.map((d) => (
              <option key={d} value={d} />
            ))}
          </datalist>
        </Field>
        {targetExists && <WarningNote>"{target}" already exists. Publishing alters or drops objects to match the DACPAC.</WarningNote>}
        <Checkbox checked={block} onChange={setBlock} label="Block on possible data loss" />
        <Checkbox checked={scriptOnly} onChange={setScriptOnly} label="Generate the deployment script only (don't change the database)" />
        <div className="flex justify-end gap-2">
          <button className="btn-ghost" onClick={close}>
            Cancel
          </button>
          <button className="btn-primary" disabled={!valid} onClick={() => void start()}>
            {scriptOnly ? "Script..." : "Import"}
          </button>
        </div>
      </div>
    </Modal>
  );
}

const startedJobs = new Set<string>();

/** Streams SqlPackage output; Cancel kills the process, then becomes Close. */
function RunDialog() {
  const run = useUiStore((s) => s.sqlpackageRun)!;
  const close = useUiStore((s) => s.closeSqlPackageRun);
  const [lines, setLines] = useState<string[]>([]);
  const [status, setStatus] = useState<string | null>(null);
  const finished = status !== null;
  const scroll = useRef<HTMLPreElement>(null);

  // Listen on every mount (StrictMode mounts twice); start the process once per job.
  useEffect(() => {
    const unlisten = listen<SqlPackageLine>("sqlpackage:output", (e) => {
      if (e.payload.jobId === run.jobId) setLines((l) => [...l, e.payload.line]);
    });
    if (!startedJobs.has(run.jobId)) {
      startedJobs.add(run.jobId);
      void ipc
        .runSqlpackage(run.jobId, run.args)
        .then((code) => {
          const ok = code === 0;
          setStatus(code === null ? "Cancelled." : ok ? "Completed successfully." : `SqlPackage exited with code ${code}.`);
          run.onDone?.(ok);
        })
        .catch((e) => {
          setLines((l) => [...l, errorMessage(e)]);
          setStatus("Failed to start SqlPackage.");
          run.onDone?.(false);
        });
    }
    return () => void unlisten.then((u) => u());
  }, [run]);

  useEffect(() => {
    scroll.current?.scrollTo({ top: scroll.current.scrollHeight });
  }, [lines]);

  return (
    <Modal title={run.title} onClose={() => (finished ? close() : void ipc.cancelSqlpackage(run.jobId))} wide="xl" explicitClose>
      <div className="flex flex-col gap-2 text-sm">
        <p className="text-content-secondary">{run.message}</p>
        <pre ref={scroll} className="mono h-80 overflow-auto rounded-md border border-border-light bg-page p-2 whitespace-pre-wrap text-content-secondary">
          {lines.join("\n")}
        </pre>
        <div className="flex items-center gap-3">
          <span className={`flex-1 text-xs ${status?.startsWith("Completed") ? "text-good" : finished ? "text-warning" : "text-content-muted"}`}>{status ?? "Running…"}</span>
          {finished ? (
            <button className="btn-primary" onClick={close}>
              Close
            </button>
          ) : (
            <button className="btn-ghost" onClick={() => void ipc.cancelSqlpackage(run.jobId)}>
              Cancel
            </button>
          )}
        </div>
      </div>
    </Modal>
  );
}
