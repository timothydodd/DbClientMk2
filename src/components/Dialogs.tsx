import { useEffect, useState } from "react";
import { AlertOctagon, AlertTriangle, CheckCircle2, Info, KeyRound, X } from "lucide-react";
import * as ipc from "../lib/ipc";
import { useToastStore } from "../stores/toast";
import { useUiStore, type ConfirmRequest, type PromptRequest } from "../stores/ui";
import { Modal } from "./ui";

function Confirm({ req }: { req: ConfirmRequest }) {
  const resolve = useUiStore((s) => s.resolveConfirm);
  return (
    <Modal title={req.title} onClose={() => resolve(req.id, false)}>
      <div className="whitespace-pre-wrap text-sm text-content-secondary">{req.message}</div>
      <div className="mt-4 flex justify-end gap-2">
        {!req.alert && (
          <button className="btn-ghost" onClick={() => resolve(req.id, false)}>
            Cancel
          </button>
        )}
        <button autoFocus className={req.danger ? "btn-danger" : "btn-primary"} onClick={() => resolve(req.id, true)}>
          {req.okText}
        </button>
      </div>
    </Modal>
  );
}

function Prompt({ req }: { req: PromptRequest }) {
  const resolve = useUiStore((s) => s.resolvePrompt);
  const [value, setValue] = useState(req.initial);
  return (
    <Modal title={req.title} onClose={() => resolve(req.id, null)}>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (value.trim()) resolve(req.id, value.trim());
        }}
      >
        <label className="field-label">{req.label}</label>
        <input autoFocus className="field w-full" value={value} placeholder={req.placeholder} onChange={(e) => setValue(e.target.value)} onFocus={(e) => e.target.select()} />
        <div className="mt-4 flex justify-end gap-2">
          <button type="button" className="btn-ghost" onClick={() => resolve(req.id, null)}>
            Cancel
          </button>
          <button type="submit" className="btn-primary" disabled={!value.trim()}>
            {req.okText}
          </button>
        </div>
      </form>
    </Modal>
  );
}

export function ConfirmDialogs() {
  const confirms = useUiStore((s) => s.confirms);
  const prompts = useUiStore((s) => s.prompts);
  return (
    <>
      {confirms.map((c) => (
        <Confirm key={c.id} req={c} />
      ))}
      {prompts.map((p) => (
        <Prompt key={p.id} req={p} />
      ))}
    </>
  );
}

/** Unknown SSH host key: show the fingerprints and ask whether to trust (and pin) it. */
export function HostKeyDialog() {
  const req = useUiStore((s) => s.hostKeys[0]);
  const pop = useUiStore((s) => s.popHostKey);
  useEffect(() => {
    // The engine gives up after five minutes; mirror that so the dialog can't outlive its request.
    if (!req) return;
    const t = setTimeout(() => pop(req.requestId), 300_000);
    return () => clearTimeout(t);
  }, [req, pop]);
  if (!req) return null;
  const answer = (trust: boolean) => {
    void ipc.answerHostKey(req.requestId, trust);
    pop(req.requestId);
  };
  return (
    <Modal title="Unknown SSH host" onClose={() => answer(false)} explicitClose>
      <div className="flex flex-col gap-3 text-sm">
        <div className="flex items-start gap-2 text-content-secondary">
          <KeyRound size={18} className="mt-0.5 shrink-0 text-warning" />
          <span>
            The authenticity of host <b className="text-content">{req.host}</b> (port {req.port}) can't be established.
          </span>
        </div>
        <div className="rounded-md bg-muted px-3 py-2">
          <div className="text-xs text-content-muted">{req.keyType} key fingerprint:</div>
          <div className="mono mt-1 break-all text-content">{req.fingerprintSha256}</div>
          {req.fingerprintMd5 && <div className="mono mt-1 break-all text-content-muted">{req.fingerprintMd5}</div>}
        </div>
        <p className="text-xs text-content-muted">Compare it with the fingerprint your server administrator gave you. Trusting it saves the key, so you are only asked once; a different key later is refused.</p>
        <div className="flex justify-end gap-2">
          <button className="btn-ghost" onClick={() => answer(false)}>
            Cancel
          </button>
          <button className="btn-primary" onClick={() => answer(true)}>
            Trust
          </button>
        </div>
      </div>
    </Modal>
  );
}

export function Toasts() {
  const { toasts, dismiss } = useToastStore();
  return (
    <div className="pointer-events-none fixed bottom-9 right-4 z-[60] flex w-80 flex-col gap-2">
      {toasts.map((t) => {
        const Icon = t.kind === "success" ? CheckCircle2 : t.kind === "error" ? AlertOctagon : Info;
        const color = t.kind === "success" ? "text-good" : t.kind === "error" ? "text-critical" : "text-info";
        return (
          <div key={t.id} className="card pointer-events-auto flex items-start gap-2 px-3 py-2.5 text-sm shadow-[var(--shadow-md)]">
            <Icon size={16} className={`mt-0.5 shrink-0 ${color}`} />
            <span className="min-w-0 flex-1 break-words text-content">{t.message}</span>
            <button className="text-content-muted hover:text-content" title="Dismiss" aria-label="Dismiss" onClick={() => dismiss(t.id)}>
              <X size={14} />
            </button>
          </div>
        );
      })}
    </div>
  );
}

export function WarningNote({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex items-start gap-2 rounded-md bg-muted px-3 py-2 text-xs text-content-secondary">
      <AlertTriangle size={14} className="mt-0.5 shrink-0 text-warning" />
      <span className="min-w-0 break-words">{children}</span>
    </div>
  );
}
