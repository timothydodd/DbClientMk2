# CLAUDE.md

Guidance for working on **dbclient**, a desktop SQL client (SQL Server / MySQL / SQLite). Tauri 2 +
Rust backend, React + TypeScript frontend. The UI and conventions follow `../portside-lite`; the
features follow the earlier Avalonia app in `../dbclient`, which this replaces.

## Build & verify

> **Run dev/prod builds from Windows, not WSL.** `npm run tauri dev` / `tauri build` need the MSVC
> toolchain + Windows Node. WSL is for editing and checks only.

**In WSL (checks only):**
```bash
export CARGO_TARGET_DIR=$HOME/.cargo-target/dbclient
cargo test -p dbclient-core -p dbclient-store -p dbclient-data -p dbclient-engine
scripts/check-tauri-wsl.sh        # type-checks src-tauri using stub pkg-config files
npx tsc --noEmit
npx vite build
```

- `src-tauri` can't *build* in WSL (no GTK/webkit libs), but `scripts/check-tauri-wsl.sh` gives
  it a real `cargo check`.
- Running `npm install` from WSL swaps in Linux native binaries (rollup, tailwind oxide). Run
  `npm install` again on Windows before building.
- Vite's file watcher doesn't see edits on `/mnt/f` from WSL. Restart the dev server to pick them up.
- TLS/crypto: MySQL and SSH use **rustls with ring**; SQL Server uses the platform TLS (schannel on
  Windows, OpenSSL only for Linux dev checks). Don't let `aws-lc-rs` into the tree (check with
  `cargo tree -i aws-lc-rs`): it needs cmake/nasm on Windows.

## Architecture

```
crates/dbclient-core     pure, unit-tested: models.rs (DTOs), intellisense/ (parser + provider),
                         formatter.rs, update_sql.rs, scripts.rs, export.rs, keywords.rs,
                         name_colors.rs, tree_filter.rs, sqlpackage.rs (arg builders)
crates/dbclient-data     DbProvider trait + drivers: sqlserver.rs (tiberius), mysql.rs
                         (mysql_async), sqlite.rs (rusqlite), ssh.rs (russh tunnel, TOFU host
                         keys), messages.rs (captures SQL Server PRINT via tracing)
crates/dbclient-store    ~/.dbclient: state.rs (state.json, connections/*.json, history.json),
                         secrets.rs (DPAPI / AES-GCM), logger.rs, paths.rs
crates/dbclient-engine   Workspace: sessions per connection, schema cache + IntelliSense,
                         execute/cancel by job id, host-key prompts, SqlPackage runner
src-tauri                thin: commands.rs wraps the engine; lib.rs forwards EventSink to events
src                      React app: components/, stores/ (zustand), lib/, hooks/
```

**Layering rule:** only `src-tauri` imports Tauri. Put testable logic in the crates.

## Key conventions

- **Files on disk are the old app's.** `~/.dbclient/state.json` (camelCase, `version`),
  `connections/<id>.json`, `history.json` (PascalCase, as System.Text.Json wrote it), `known_hosts`
  (`host:port keytype base64`), `log.txt`. Enums are read as the names this app writes *and* the
  integers the .NET app wrote (`models.rs` `de` module). Credentials keep the `dpapi:` / `aesgcm:`
  formats with the same DPAPI entropy, so saved passwords still open.
- **Secrets never reach the webview.** `AppState`/`ConnectionConfig` go out `redacted()` with
  `SAVED_SECRET` in place of each set secret; `upsert_connection` / `save_app_state` /
  `test_connection` call `restore_secrets` to put saved values back by connection id.
- **Per-database query tab sets.** `src/lib/tabs.ts` holds the pure logic (mirrors the old
  `ConnectionTabViewModel`): the active database's tabs are `sets.tabs`, the rest are stashed by
  database; the `""` key is tabs made before any database was active and they migrate into the
  first one. `switchDatabase` calls `activateDatabaseTabs`; persistence uses `collectAllTabStates`
  + `activeTabIdsByDatabase`.
- **Query execution.** `execute_query(id, job_id, database, sql)` opens a fresh connection per
  query; `cancel_query(job_id)` cancels the token and the future is dropped. SQL Server affected
  rows come from a follow-up `SELECT @@ROWCOUNT` (tiberius hides DONE counts); PRINT output is
  captured by `messages.rs` from tiberius' DEBUG tracing events scoped to the query's task.
- **IntelliSense** lives in Rust (`dbclient-core/src/intellisense/`): `lexer.rs` tokenizes,
  `scope.rs` finds statements, CTEs, query blocks (subqueries, derived tables), sources with
  aliases, clauses, `DECLARE`d variables and temp tables; `parser.rs` classifies the caret;
  `provider.rs` resolves columns through CTEs/derived/temp tables, ranks with `match_score`
  (prefix > camel-hump > word boundary > substring > subsequence) plus per-session recency
  (`completion_accepted`), and adds actions: `*` expansion, FK-based `JOIN ... ON` and ON
  conditions, INSERT column/VALUES templates, snippets (`snippets.rs`). `hover` and
  `diagnostics` back the editor's tooltips and underlines. Items carry `insertText` (with
  `$0` / `${n:text}` fields the editor turns into CodeMirror snippets) and `replaceFrom`.
  Schema input: `DbDatabase` now includes `functions` and `foreignKeys`; procedures carry
  `parameters`. Add a parser case as a unit test in `scope.rs`/`parser.rs` before changing
  classification.
- **DTOs are camelCase** (serde) and mirrored by hand in `src/lib/types.ts`; keep them in sync.
  IPC wrappers live in `src/lib/ipc.ts`.
- **Events to the UI:** `ssh:host-key` (answer with `answer_host_key`), `sqlpackage:output`.
- **Theming:** Portside's tokens in `src/index.css` (dark default) plus `[data-theme="dracula"]`
  (the old app's Dracula palette) and `[data-theme="light"]`. Use the semantic utilities
  (`bg-surface`, `text-content`, `border-border`, …) and component classes (`btn-*`, `field`,
  `card`, `menu`, `panel-header`, `tint-*`). Grid data-type colors are `--dt-*`, completion kinds
  `--kind-*`, editor syntax `--syn-*`. Name-derived colors (database badges, connection tabs) are in
  `src/lib/colors.ts` and `name_colors.rs` and must agree.
- **Dialogs:** `ui.confirm` / `ui.alert` / `ui.prompt` (`stores/ui.ts`, promise-based, rendered by
  `components/Dialogs.tsx`), never `window.confirm`. Context menus go through `openContextMenu`.
- **Zustand selectors must return existing references.** Select raw fields and derive with
  `useMemo`; `selectedConnection` / `activeQueryTab` are the shared selectors.
- **Licenses.** MIT (`LICENSE`). `THIRD_PARTY_NOTICES.md` is generated by
  `scripts/third-party-notices.py` from the lockfiles; rerun it after any dependency change (CI runs
  `--check`). Releases ship `THIRD_PARTY_LICENSES.txt` (`--texts`).
- **Icons:** Lucide (`lucide-react`), never custom SVG paths.
