export function errorMessage(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}

export function fmtCount(n: number): string {
  return n.toLocaleString();
}

export function fmtMs(ms: number): string {
  return `${Math.round(ms)}ms`;
}

/** "just now", "5m ago", "3h ago", "2d ago", else "Mar 5". */
export function fmtAgo(iso: string, now = Date.now()): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "";
  const diff = now - t;
  const m = diff / 60_000;
  if (m < 1) return "just now";
  if (m < 60) return `${Math.floor(m)}m ago`;
  const h = m / 60;
  if (h < 24) return `${Math.floor(h)}h ago`;
  const d = h / 24;
  if (d < 7) return `${Math.floor(d)}d ago`;
  return new Date(t).toLocaleDateString([], { month: "short", day: "numeric" });
}

export function fmtDateTime(iso: string): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return iso;
  return new Date(t).toLocaleString([], { dateStyle: "short", timeStyle: "short" });
}

/** First line, whitespace collapsed, cut to `max` chars with an ellipsis. */
export function oneLine(text: string, max: number): string {
  const s = text.replace(/\r/g, "").replace(/\n/g, " ").replace(/\s+/g, " ").trim();
  return s.length > max ? s.slice(0, max - 1) + "…" : s;
}

export function fileName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

export function fileStem(path: string): string {
  const name = fileName(path);
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(0, dot) : name;
}
