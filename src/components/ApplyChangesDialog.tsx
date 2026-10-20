import { useState } from "react";
import { useUiStore } from "../stores/ui";
import { WarningNote } from "./Dialogs";
import { Modal } from "./ui";

/** Preview (and optionally edit) the generated UPDATE script before it runs. */
export default function ApplyChangesDialog() {
  const req = useUiStore((s) => s.applyChanges);
  const close = useUiStore((s) => s.closeApplyChanges);
  if (!req) return null;
  return <Body key={req.sql} sql={req.sql} expected={req.expected} resolve={(r) => (close(), req.resolve(r))} />;
}

function Body({ sql, expected, resolve }: { sql: string; expected: number; resolve: (r: { execute: boolean; sql: string }) => void }) {
  const [text, setText] = useState(sql);
  const edited = text !== sql;
  return (
    <Modal title="Apply Changes" onClose={() => resolve({ execute: false, sql: text })} wide="xl" explicitClose>
      <div className="flex flex-col gap-3 text-sm">
        <p className="text-content-secondary">Review the UPDATE statements generated from your edits. You can change the script before running it.</p>
        {expected > 0 && <div className="text-xs text-content-muted">{expected} UPDATE statement(s), each expected to affect exactly 1 row.</div>}
        <textarea className="field mono h-72 w-full resize-y whitespace-pre" spellCheck={false} value={text} onChange={(e) => setText(e.target.value)} />
        {edited && <WarningNote>The script was edited. The row-count check is skipped; the script runs exactly as shown.</WarningNote>}
        <div className="flex justify-end gap-2">
          <button className="btn-ghost" onClick={() => resolve({ execute: false, sql: text })}>
            Cancel
          </button>
          <button className="btn-primary" onClick={() => resolve({ execute: true, sql: text })}>
            Execute
          </button>
        </div>
      </div>
    </Modal>
  );
}
