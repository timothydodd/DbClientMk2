import { useEffect, useRef, type ReactNode } from "react";
import { Loader2, X } from "lucide-react";
import { isWindows } from "./WindowControls";

export function Spinner({ size = 16 }: { size?: number }) {
  return <Loader2 size={size} className="animate-spin text-content-muted" />;
}

export function EmptyState({ icon, title, children }: { icon?: ReactNode; title: string; children?: ReactNode }) {
  return (
    <div className="flex flex-col items-center justify-center gap-2 px-6 py-10 text-center">
      {icon && <div className="text-content-muted">{icon}</div>}
      <div className="text-sm font-medium text-content">{title}</div>
      {children && <div className="max-w-xs text-xs text-content-muted">{children}</div>}
    </div>
  );
}

export function Kbd({ children }: { children: ReactNode }) {
  return <kbd className="mono rounded border border-border bg-raised px-1.5 py-0.5 text-[11px] text-content-secondary">{children}</kbd>;
}

// Open overlays in the order they opened. Esc goes to the last one only.
const escStack: { current: (() => void) | null }[] = [];
let escListening = false;

export function useEscape(onClose: () => void, enabled = true) {
  const handler = useRef<(() => void) | null>(null);
  handler.current = enabled ? onClose : null;
  useEffect(() => {
    if (!escListening) {
      escListening = true;
      window.addEventListener("keydown", (e) => {
        if (e.key === "Escape" && escStack.length) {
          escStack[escStack.length - 1]?.current?.();
          e.stopPropagation();
        }
      }, true);
    }
    escStack.push(handler);
    return () => void escStack.splice(escStack.indexOf(handler), 1);
  }, []);
}

/** True while any modal overlay is open (so global shortcuts like Esc-cancel stay out of the way). */
export const overlaysOpen = () => escStack.length > 0;

export function Modal({
  title,
  onClose,
  children,
  wide = false,
  explicitClose = false,
  footer,
}: {
  title: string;
  onClose: () => void;
  children: ReactNode;
  wide?: boolean | "xl";
  /** Only its own buttons close it: no Esc, no click outside. */
  explicitClose?: boolean;
  footer?: ReactNode;
}) {
  useEscape(onClose, !explicitClose);
  const width = wide === "xl" ? "w-[min(900px,96vw)]" : wide ? "w-[min(640px,94vw)]" : "w-[440px]";
  return (
    <div className={`fixed inset-x-0 bottom-0 z-50 flex items-center justify-center bg-black/40 ${isWindows ? "top-8" : "top-0"}`} onMouseDown={explicitClose ? undefined : onClose}>
      <div className={`card flex max-h-[90vh] flex-col overflow-hidden shadow-[var(--shadow-md)] ${width}`} onMouseDown={(e) => e.stopPropagation()}>
        <header className="flex items-center justify-between border-b border-border-light px-4 py-2.5">
          <h2 className="text-sm font-semibold text-content">{title}</h2>
          <button className="btn-icon" onClick={onClose} title="Close" aria-label="Close">
            <X size={14} />
          </button>
        </header>
        <div className="min-h-0 flex-1 overflow-auto px-4 py-3">{children}</div>
        {footer && <footer className="flex items-center justify-end gap-2 border-t border-border-light bg-raised px-4 py-2.5">{footer}</footer>}
      </div>
    </div>
  );
}

export function Field({ label, children, hint }: { label: ReactNode; children: ReactNode; hint?: ReactNode }) {
  return (
    <label className="block">
      <span className="field-label">{label}</span>
      {children}
      {hint && <span className="mt-1 block text-[11px] text-content-muted">{hint}</span>}
    </label>
  );
}

export function Checkbox({ checked, onChange, label }: { checked: boolean; onChange: (v: boolean) => void; label: ReactNode }) {
  return (
    <label className="flex items-center gap-2 text-sm text-content-secondary">
      <input type="checkbox" className="accent-brand" checked={checked} onChange={(e) => onChange(e.target.checked)} />
      {label}
    </label>
  );
}
