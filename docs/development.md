# Development

dbclient is a Tauri 2 app: a Rust backend split into Tauri-free crates, and a React + TypeScript
frontend. [CLAUDE.md](../CLAUDE.md) describes the architecture and conventions in detail.

```
crates/dbclient-core     pure logic: models/DTOs, SQL IntelliSense, formatter, UPDATE generator,
                         script templates, exports
crates/dbclient-data     database drivers: SQL Server (tiberius), MySQL (mysql_async),
                         SQLite (rusqlite), SSH tunnels (russh)
crates/dbclient-store    ~/.dbclient on disk: state, per-connection tabs, history, log,
                         encrypted credentials
crates/dbclient-engine   the workspace: sessions, schema cache, query execution, SqlPackage
src-tauri                thin Tauri layer: commands, events
src                      React app
```

## Build and run

Build and run on **Windows** (it needs the MSVC toolchain and Windows Node):

```powershell
npm install
npm run tauri dev
```

**UI-only work.** `npm run dev` serves the UI at http://localhost:1470 with a mocked backend and
fixture data (`src/dev/mockTauri.ts`, dev builds only), so it opens in a normal browser.

## Checks

```bash
cargo test -p dbclient-core -p dbclient-store -p dbclient-data -p dbclient-engine
npx tsc --noEmit
```

**WSL.** WSL works for editing and checks, but not for building the app, because the Tauri crate
needs the Windows toolchain. `scripts/check-tauri-wsl.sh` type-checks it there with stubbed system
libraries. CLAUDE.md lists the other WSL caveats.

**Dependencies.** TLS and crypto use `ring` (MySQL, SSH) and the platform's own TLS (schannel) for
SQL Server. Don't add dependencies that pull in `aws-lc-rs` or OpenSSL: they need cmake, nasm and
perl to build on Windows.

## Third-party notices

`THIRD_PARTY_NOTICES.md` is generated from `Cargo.lock` and `package-lock.json`. After adding,
removing or updating dependencies, regenerate it:

```bash
python3 scripts/third-party-notices.py
```

CI fails if it's out of date. Release builds use the same script with `--texts` to produce
`THIRD_PARTY_LICENSES.txt` (the full license texts), which ships with the installer and the
portable zip.
