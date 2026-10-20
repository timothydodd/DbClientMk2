import { useEffect, useState } from "react";
import { openPath, openUrl } from "@tauri-apps/plugin-opener";
import { copyText } from "../lib/clipboard";
import * as ipc from "../lib/ipc";
import { SHORTCUTS } from "../lib/shortcuts";
import type { AppInfo } from "../lib/types";
import { toast } from "../stores/toast";
import { useUiStore } from "../stores/ui";
import { REPO_URL } from "./MenuBar";
import { Kbd, Modal } from "./ui";

export default function AboutDialog() {
  const page = useUiStore((s) => s.about);
  const open = useUiStore((s) => s.openAbout);
  const close = useUiStore((s) => s.closeAbout);
  const [info, setInfo] = useState<AppInfo | null>(null);
  useEffect(() => {
    if (page) void ipc.appInfo().then(setInfo).catch(() => setInfo(null));
  }, [page]);
  if (!page) return null;

  const copyInfo = async () => {
    if (!info) return;
    const ok = await copyText(`dbclient ${info.version}\n${info.os}\nTauri 2 / React\nLog: ${info.logFile}`);
    if (ok) toast.success("Copied");
  };

  let section: string | null = null;
  return (
    <Modal title={page === "about" ? "About dbclient" : "Keyboard Shortcuts"} onClose={close} wide>
      <div className="mb-3 flex gap-1 border-b border-border-light">
        <button className={`navtab ${page === "about" ? "navtab-active" : ""}`} onClick={() => open("about")}>
          About
        </button>
        <button className={`navtab ${page === "shortcuts" ? "navtab-active" : ""}`} onClick={() => open("shortcuts")}>
          Shortcuts
        </button>
      </div>
      {page === "about" ? (
        <div className="flex flex-col gap-3 text-sm">
          <div className="flex items-center gap-3">
            <img src="/icon.png" alt="" className="h-12 w-12" />
            <div>
              <div className="text-base font-semibold text-content">dbclient</div>
              <div className="text-xs text-content-muted">Version {info?.version ?? "…"}</div>
            </div>
          </div>
          <p className="text-content-secondary">A SQL client for SQL Server (including Azure SQL), MySQL / MariaDB and SQLite, over a direct connection or an SSH tunnel.</p>
          <dl className="grid grid-cols-[120px_1fr] gap-y-1 text-xs">
            <dt className="text-content-muted">Runtime</dt>
            <dd className="text-content-secondary">Tauri 2 (Rust) + React</dd>
            <dt className="text-content-muted">OS</dt>
            <dd className="text-content-secondary">{info?.os}</dd>
            <dt className="text-content-muted">Data folder</dt>
            <dd className="mono break-all text-content-secondary">{info?.dataDir}</dd>
            <dt className="text-content-muted">Log file</dt>
            <dd className="mono break-all text-content-secondary">{info?.logFile}</dd>
            <dt className="text-content-muted">SqlPackage</dt>
            <dd className="mono break-all text-content-secondary">{info?.sqlpackagePath ?? "not found"}</dd>
          </dl>
          <div className="flex flex-wrap gap-2">
            <button className="btn-ghost" onClick={() => void openUrl(REPO_URL)}>
              GitHub
            </button>
            <button className="btn-ghost" onClick={() => void openUrl(`${REPO_URL}/issues`)}>
              Report an issue
            </button>
            <button className="btn-ghost" onClick={() => info && void openPath(info.dataDir)}>
              Open log folder
            </button>
            <button className="btn-ghost" onClick={() => void copyInfo()}>
              Copy info
            </button>
          </div>
          <p className="text-xs text-content-muted">MIT License. Copyright (c) 2026 Tim Dodd. Third-party components are listed in THIRD_PARTY_NOTICES.md.</p>
        </div>
      ) : (
        <div className="flex flex-col gap-1 text-sm">
          {SHORTCUTS.map((s, i) => {
            const header = s.section !== section ? s.section : null;
            section = s.section;
            return (
              <div key={i}>
                {header && <div className="card-title mt-3 mb-1 text-accent">{header}</div>}
                <div className="grid grid-cols-[220px_1fr] items-center gap-2 py-0.5">
                  <span>
                    {s.keys.split(" / ").map((k, j) => (
                      <span key={j}>
                        {j > 0 && <span className="mx-1 text-content-muted">/</span>}
                        <Kbd>{k}</Kbd>
                      </span>
                    ))}
                  </span>
                  <span className="text-content-secondary">{s.action}</span>
                </div>
              </div>
            );
          })}
        </div>
      )}
    </Modal>
  );
}
