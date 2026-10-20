import { useEffect, useMemo, useRef } from "react";
import CodeMirror, { type ReactCodeMirrorRef } from "@uiw/react-codemirror";
import { EditorView, hoverTooltip, keymap } from "@codemirror/view";
import { EditorState, type Extension } from "@codemirror/state";
import { autocompletion, snippet, type Completion, type CompletionContext, type CompletionResult } from "@codemirror/autocomplete";
import { linter, type Diagnostic } from "@codemirror/lint";
import { openSearchPanel, search, searchKeymap } from "@codemirror/search";
import { MSSQL, MySQL, SQLite, sql } from "@codemirror/lang-sql";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags } from "@lezer/highlight";
import * as ipc from "../lib/ipc";
import type { CompletionItem, ConnectionType } from "../lib/types";
import { useUiStore } from "../stores/ui";
import { registerEditor, useWorkspace, type QueryTab } from "../stores/workspace";

const TRIGGER_KEYWORDS = new Set([
  "SELECT", "FROM", "WHERE", "JOIN", "INNER", "LEFT", "RIGHT", "CROSS", "FULL", "ON", "AND", "OR", "INSERT", "INTO", "UPDATE", "SET",
  "DELETE", "HAVING", "ORDER", "GROUP", "BY", "AS", "DISTINCT", "TOP", "EXEC", "EXECUTE", "CALL", "USING", "APPLY", "VALUES", "LIKE",
  "IN", "NOT", "EXISTS", "WHEN", "THEN", "ELSE", "CASE", "RETURNING", "LIMIT", "OFFSET",
]);

const highlight = syntaxHighlighting(
  HighlightStyle.define([
    { tag: tags.keyword, class: "tok-keyword" },
    { tag: [tags.string, tags.special(tags.string)], class: "tok-string" },
    { tag: [tags.comment, tags.lineComment, tags.blockComment], class: "tok-comment" },
    { tag: [tags.number, tags.integer, tags.float], class: "tok-number" },
    { tag: [tags.function(tags.variableName), tags.standard(tags.name)], class: "tok-function" },
    { tag: tags.typeName, class: "tok-type" },
    { tag: [tags.operator, tags.compareOperator, tags.arithmeticOperator, tags.logicOperator], class: "tok-operator" },
  ]),
);

function dialect(type: ConnectionType) {
  return type === "sqlServer" ? MSSQL : type === "mySql" ? MySQL : SQLite;
}

/** Our `$0` / `${n:text}` markers → CodeMirror's `${}` / `${n:text}` template syntax. */
function toSnippetTemplate(text: string): string {
  return text.replace(/\$0/g, "${}");
}

/** Toggle `-- ` on every line touched by the selection; strip it when all non-blank lines are commented. */
function toggleLineComment(view: EditorView) {
  const { state } = view;
  const range = state.selection.main;
  const from = state.doc.lineAt(range.from).number;
  let to = state.doc.lineAt(range.to).number;
  if (range.to > range.from && state.doc.lineAt(range.to).from === range.to) to -= 1;
  const lines = [];
  for (let n = from; n <= to; n++) lines.push(state.doc.line(n));
  const nonBlank = lines.filter((l) => l.text.trim() !== "");
  const allCommented = nonBlank.length > 0 && nonBlank.every((l) => l.text.trimStart().startsWith("--"));
  const changes = [];
  for (const line of lines) {
    if (allCommented) {
      const idx = line.text.indexOf("--");
      if (idx < 0) continue;
      const len = line.text[idx + 2] === " " ? 3 : 2;
      changes.push({ from: line.from + idx, to: line.from + idx + len });
    } else {
      if (line.text.trim() === "" && lines.length > 1) continue;
      const indent = line.text.length - line.text.trimStart().length;
      changes.push({ from: line.from + indent, insert: "-- " });
    }
  }
  if (changes.length) view.dispatch({ changes });
  return true;
}

/** Map a backend completion to a CodeMirror one, with snippet fields and wider replacements. */
function toCompletion(item: CompletionItem, index: number, connId: string): Completion {
  const text = item.insertText ?? item.text;
  const accepted = () => void ipc.completionAccepted(connId, item.text).catch(() => undefined);
  const base: Completion = {
    label: item.text,
    type: item.type,
    detail: item.description ?? undefined,
    // Backend order wins: boost by position so CodeMirror's own scoring doesn't reshuffle it.
    boost: 99 - Math.min(99, index),
  };
  const hasFields = /\$\{|\$0/.test(text);
  if (hasFields) {
    const apply = snippet(toSnippetTemplate(text));
    base.apply = (view, completion, from, to) => {
      accepted();
      apply(view, completion, item.replaceFrom ?? from, to);
    };
  } else if (item.replaceFrom !== undefined || item.insertText !== undefined) {
    base.apply = (view, _completion, from, to) => {
      accepted();
      const start = item.replaceFrom ?? from;
      view.dispatch({ changes: { from: start, to, insert: text }, selection: { anchor: start + text.length } });
    };
  } else {
    base.apply = (view, _completion, from, to) => {
      accepted();
      view.dispatch({ changes: { from, to, insert: text }, selection: { anchor: from + text.length } });
    };
  }
  return base;
}

export default function SqlEditor({ connId, connType, database, tab, fontSize, wordWrap }: { connId: string; connType: ConnectionType; database: string; tab: QueryTab; fontSize: number; wordWrap: boolean }) {
  const ref = useRef<ReactCodeMirrorRef>(null);
  const ws = useWorkspace();
  const setFocusedPane = useUiStore((s) => s.setFocusedPane);
  const tabId = tab.id;
  const dbRef = useRef(database);
  dbRef.current = database;

  // The store needs the live editor for execute-selection, set-text, etc.
  useEffect(() => {
    const api = {
      getText: () => ref.current?.view?.state.doc.toString() ?? "",
      getSelection: () => {
        const v = ref.current?.view;
        if (!v) return "";
        const r = v.state.selection.main;
        return r.empty ? "" : v.state.sliceDoc(r.from, r.to);
      },
      setText: (text: string) => {
        const v = ref.current?.view;
        if (!v || v.state.doc.toString() === text) return;
        v.dispatch({ changes: { from: 0, to: v.state.doc.length, insert: text } });
      },
      toggleComment: () => {
        const v = ref.current?.view;
        if (v) toggleLineComment(v);
      },
      focus: () => ref.current?.view?.focus(),
      openSearch: () => {
        const v = ref.current?.view;
        if (v) openSearchPanel(v);
      },
    };
    registerEditor(tabId, api);
    return () => registerEditor(tabId, null);
  }, [tabId]);

  const extensions = useMemo<Extension[]>(() => {
    const source = async (ctx: CompletionContext): Promise<CompletionResult | null> => {
      const before = ctx.state.sliceDoc(0, ctx.pos);
      const last = before[before.length - 1] ?? "";
      const word = ctx.matchBefore(/[@#]*[\w$]*$/);
      if (!ctx.explicit) {
        // Identifier chars, a dot, `(`, `@`/`#`, and a space after a keyword (mirrors the original editor).
        let trigger = /[\w@#$]/.test(last) || last === "." || last === "(";
        if (last === " ") {
          const m = /(\w+)\s+$/.exec(before);
          trigger = (!!m && TRIGGER_KEYWORDS.has(m[1].toUpperCase())) || /,\s+$/.test(before);
        }
        if (!trigger) return null;
      }
      const items = await ipc.getCompletions(connId, dbRef.current, ctx.state.doc.toString(), ctx.pos).catch(() => []);
      if (!items.length) return null;
      return {
        from: word?.from ?? ctx.pos,
        options: items.map((it, i) => toCompletion(it, i, connId)),
        // Re-query on every keystroke (the backend ranks); no client-side filtering.
        filter: false,
      };
    };

    const hover = hoverTooltip(
      async (view, pos) => {
        const h = await ipc.getHover(connId, dbRef.current, view.state.doc.toString(), pos).catch(() => null);
        if (!h) return null;
        return {
          pos: h.start,
          end: h.end,
          above: true,
          create: () => {
            const dom = document.createElement("div");
            const title = document.createElement("div");
            title.className = "hover-title";
            title.textContent = h.title;
            const detail = document.createElement("div");
            detail.className = "hover-detail";
            detail.textContent = h.detail;
            dom.append(title, detail);
            return { dom };
          },
        };
      },
      { hoverTime: 350 },
    );

    const lint = linter(
      async (view): Promise<Diagnostic[]> => {
        const ds = await ipc.getDiagnostics(connId, dbRef.current, view.state.doc.toString()).catch(() => []);
        const len = view.state.doc.length;
        return ds
          .filter((d) => d.start < d.end && d.end <= len)
          .map((d) => ({ from: d.start, to: d.end, severity: d.severity, message: d.message }));
      },
      { delay: 700 },
    );

    const ext: Extension[] = [
      sql({ dialect: dialect(connType), upperCaseKeywords: true }),
      highlight,
      search({ top: true }),
      keymap.of([
        ...searchKeymap,
        { key: "Mod-/", run: toggleLineComment },
        { key: "F5", run: () => (void useWorkspace.getState().execute(), true) },
        { key: "Mod-Enter", run: () => (void useWorkspace.getState().execute(), true) },
      ]),
      autocompletion({ override: [source], activateOnTyping: true, maxRenderedOptions: 80, icons: true, defaultKeymap: true, closeOnBlur: true, activateOnTypingDelay: 60 }),
      hover,
      lint,
      EditorView.theme({ "&": { fontSize: `${fontSize}px` } }),
      EditorView.updateListener.of((u) => {
        if (u.selectionSet || u.docChanged) {
          const pos = u.state.selection.main.head;
          const line = u.state.doc.lineAt(pos);
          useWorkspace.getState().setCursor(connId, tabId, line.number, pos - line.from + 1);
        }
      }),
      EditorState.allowMultipleSelections.of(true),
    ];
    if (wordWrap) ext.push(EditorView.lineWrapping);
    return ext;
  }, [connId, connType, tabId, fontSize, wordWrap]);

  return (
    <div className="h-full min-h-0" onFocus={() => setFocusedPane("editor")}>
      <CodeMirror
        ref={ref}
        value={tab.queryText}
        height="100%"
        className="h-full"
        theme="none"
        extensions={extensions}
        onChange={(v) => ws.setQueryText(connId, tabId, v)}
        basicSetup={{ foldGutter: false, highlightActiveLine: true, autocompletion: false, searchKeymap: false, defaultKeymap: true, bracketMatching: true, closeBrackets: false, lineNumbers: true, highlightActiveLineGutter: true }}
      />
    </div>
  );
}
