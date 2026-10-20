import { useState } from "react";
import { Eye, EyeOff } from "lucide-react";
import { pickFile } from "../lib/files";
import { errorMessage } from "../lib/format";
import * as ipc from "../lib/ipc";
import { SAVED_SECRET, type ConnectionConfig, type ConnectionType, type SqlAuthMode } from "../lib/types";
import { toast } from "../stores/toast";
import { useUiStore } from "../stores/ui";
import { useWorkspace } from "../stores/workspace";
import { Checkbox, Field, Modal, Spinner } from "./ui";

function SecretInput({ value, onChange, placeholder }: { value: string; onChange: (v: string) => void; placeholder?: string }) {
  const [show, setShow] = useState(false);
  const saved = value === SAVED_SECRET;
  return (
    <div className="flex gap-1">
      <input
        type={show ? "text" : "password"}
        className="field w-full"
        value={saved ? "" : value}
        placeholder={saved ? "•••••••• (saved)" : placeholder}
        onChange={(e) => onChange(e.target.value)}
      />
      <button type="button" className="btn-icon h-auto w-8 shrink-0 border border-border" title="Show / hide" onClick={() => setShow((s) => !s)}>
        {show ? <EyeOff size={14} /> : <Eye size={14} />}
      </button>
    </div>
  );
}

/** New / edit connection. Connect saves the connection and opens it. */
export default function ConnectionDialog() {
  const config = useUiStore((s) => s.connectionDialog);
  const close = useUiStore((s) => s.closeConnectionDialog);
  if (!config) return null;
  return <Form key={config.id || "new"} initial={config} onClose={close} />;
}

function Form({ initial, onClose }: { initial: ConnectionConfig; onClose: () => void }) {
  const [c, setC] = useState<ConnectionConfig>(initial);
  const [status, setStatus] = useState<{ text: string; ok: boolean } | null>(null);
  const [testing, setTesting] = useState(false);
  const [saving, setSaving] = useState(false);
  const upsert = useWorkspace((s) => s.upsertConnection);
  const openConnection = useWorkspace((s) => s.openConnection);
  const set = (patch: Partial<ConnectionConfig>) => setC({ ...c, ...patch });
  const isNew = !initial.id;
  const isServer = c.type !== "sqlite";
  const isSqlServer = c.type === "sqlServer";
  const showLogin = isServer && (c.type !== "sqlServer" || c.authMode === "sqlLogin");

  const test = async () => {
    setTesting(true);
    setStatus({ text: "Testing...", ok: true });
    try {
      const r = await ipc.testConnection(c);
      setStatus({ text: r.message, ok: r.ok });
    } catch (e) {
      setStatus({ text: `Failed: ${errorMessage(e)}`, ok: false });
    } finally {
      setTesting(false);
    }
  };

  const connect = async () => {
    setSaving(true);
    try {
      const saved = await upsert(c);
      onClose();
      await openConnection(saved.id);
    } catch (e) {
      toast.error(errorMessage(e));
    } finally {
      setSaving(false);
    }
  };

  const save = async () => {
    setSaving(true);
    try {
      await upsert(c);
      onClose();
    } catch (e) {
      toast.error(errorMessage(e));
    } finally {
      setSaving(false);
    }
  };

  const num = (v: string, fallback: number) => {
    const n = parseInt(v, 10);
    return Number.isFinite(n) && n >= 0 ? n : fallback;
  };

  return (
    <Modal
      title={isNew ? "New Connection" : "Edit Connection"}
      onClose={onClose}
      explicitClose
      footer={
        <>
          <span className={`mr-auto min-w-0 truncate text-xs ${status?.ok === false ? "text-critical" : "text-content-secondary"}`} title={status?.text}>
            {status?.text}
          </span>
          <button className="btn-ghost" disabled={testing} onClick={() => void test()}>
            {testing ? <Spinner size={14} /> : null} Test
          </button>
          {!isNew && (
            <button className="btn-ghost" disabled={saving} onClick={() => void save()}>
              Save
            </button>
          )}
          <button className="btn-primary" disabled={saving} onClick={() => void connect()}>
            Connect
          </button>
          <button className="btn-ghost" onClick={onClose}>
            Cancel
          </button>
        </>
      }
    >
      <div className="flex flex-col gap-3">
        <Field label="Connection Type">
          <select className="field w-full" value={c.type} onChange={(e) => set({ type: e.target.value as ConnectionType })}>
            <option value="sqlServer">SQL Server</option>
            <option value="mySql">MySQL</option>
            <option value="sqlite">SQLite</option>
          </select>
        </Field>
        <Field label="Display Name (optional)">
          <input className="field w-full" value={c.displayName} placeholder="My Database" onChange={(e) => set({ displayName: e.target.value })} />
        </Field>

        {isServer && (
          <>
            <Field label="Server Address">
              <input className="field w-full" value={c.address} placeholder="localhost" onChange={(e) => set({ address: e.target.value })} />
            </Field>
            {isSqlServer && (
              <Field label="Authentication">
                <select className="field w-full" value={c.authMode} onChange={(e) => set({ authMode: e.target.value as SqlAuthMode })}>
                  <option value="sqlLogin">SQL Login</option>
                  <option value="azureDefault">Azure (use az login)</option>
                  <option value="windows">Windows Authentication</option>
                </select>
              </Field>
            )}
            {showLogin && (
              <div className="grid grid-cols-[1fr_100px] gap-3">
                <Field label="Username">
                  <input className="field w-full" value={c.user} placeholder={isSqlServer ? "sa" : "root"} onChange={(e) => set({ user: e.target.value })} />
                </Field>
                <Field label="Port">
                  <input className="field w-full" value={c.port} placeholder="default" onChange={(e) => set({ port: e.target.value })} />
                </Field>
              </div>
            )}
            {showLogin && (
              <Field label="Password">
                <SecretInput value={c.password} onChange={(v) => set({ password: v })} />
              </Field>
            )}
            {isSqlServer && c.authMode !== "sqlLogin" && (
              <Field label="Port">
                <input className="field w-full" value={c.port} placeholder="default" onChange={(e) => set({ port: e.target.value })} />
              </Field>
            )}
            {isSqlServer && (
              <div className="flex flex-col gap-1.5">
                <Checkbox checked={c.encrypt} onChange={(v) => set({ encrypt: v })} label="Encrypt connection" />
                <Checkbox checked={c.trustServerCertificate} onChange={(v) => set({ trustServerCertificate: v })} label="Trust server certificate" />
                <div className="ml-6 text-[11px] text-content-muted">Trust server certificate skips TLS certificate validation; only for self-signed dev servers.</div>
              </div>
            )}
            <Field label="Database (optional)">
              <input className="field w-full" value={c.database} placeholder="Leave blank to browse all" onChange={(e) => set({ database: e.target.value })} />
            </Field>
            <div className="grid grid-cols-2 gap-3">
              <Field label="Connection Timeout (seconds)">
                <input className="field w-full" type="number" min={1} value={c.connectionTimeout} onChange={(e) => set({ connectionTimeout: num(e.target.value, 15) })} />
              </Field>
              <Field label="Command Timeout (seconds)">
                <input className="field w-full" type="number" min={1} value={c.commandTimeout} onChange={(e) => set({ commandTimeout: num(e.target.value, 30) })} />
              </Field>
            </div>
          </>
        )}

        {c.type === "sqlite" && (
          <Field label="Database File">
            <div className="flex gap-2">
              <input className="field w-full" value={c.fileName} placeholder="/path/to/database.db" onChange={(e) => set({ fileName: e.target.value })} />
              <button
                type="button"
                className="btn-ghost shrink-0"
                onClick={() =>
                  void pickFile("Select SQLite Database", [
                    { name: "SQLite Database", extensions: ["db", "sqlite", "sqlite3", "s3db"] },
                    { name: "All Files", extensions: ["*"] },
                  ]).then((p) => p && set({ fileName: p }))
                }
              >
                Browse...
              </button>
            </div>
          </Field>
        )}

        {isServer && (
          <div className="flex flex-col gap-3 border-t border-border-light pt-3">
            <Checkbox checked={c.useSSH} onChange={(v) => set({ useSSH: v })} label="Use SSH Tunnel" />
            {c.useSSH && (
              <>
                <div className="grid grid-cols-[1fr_100px] gap-3">
                  <Field label="SSH Host">
                    <input className="field w-full" value={c.sshHost} onChange={(e) => set({ sshHost: e.target.value })} />
                  </Field>
                  <Field label="SSH Port" hint="The Port field above">
                    <input className="field w-full" value={c.port} placeholder="22" onChange={(e) => set({ port: e.target.value })} />
                  </Field>
                </div>
                <div className="grid grid-cols-2 gap-3">
                  <Field label="SSH Username">
                    <input className="field w-full" value={c.sshUser} onChange={(e) => set({ sshUser: e.target.value })} />
                  </Field>
                  <Field label="SSH Password">
                    <SecretInput value={c.sshPassword} onChange={(v) => set({ sshPassword: v })} />
                  </Field>
                </div>
                <Field label="SSH Private Key File (optional)">
                  <div className="flex gap-2">
                    <input className="field w-full" value={c.sshKeyFile} placeholder="~/.ssh/id_ed25519" onChange={(e) => set({ sshKeyFile: e.target.value })} />
                    <button
                      type="button"
                      className="btn-ghost shrink-0"
                      onClick={() =>
                        void pickFile("Select SSH Private Key", [
                          { name: "Private Key", extensions: ["pem", "key", "ppk", "*"] },
                        ]).then((p) => p && set({ sshKeyFile: p }))
                      }
                    >
                      Browse...
                    </button>
                  </div>
                </Field>
                <Field label="SSH Key Passphrase (optional)">
                  <SecretInput value={c.sshKeyPassphrase} onChange={(v) => set({ sshKeyPassphrase: v })} />
                </Field>
                <Field label="Remote Port" hint="Database port as seen from the SSH server (e.g. 1433 or 3306)">
                  <input className="field w-full" type="number" min={0} value={c.sshRemotePort} onChange={(e) => set({ sshRemotePort: num(e.target.value, 0) })} />
                </Field>
              </>
            )}
          </div>
        )}
      </div>
    </Modal>
  );
}
