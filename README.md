<p align="center">
  <img src="app-icon.png" alt="dbclient" width="140" />
</p>

# dbclient

A Windows desktop SQL client for SQL Server (including Azure SQL), MySQL / MariaDB and SQLite,
over a direct connection or an SSH tunnel. Built with Tauri 2 (Rust) and React; the Rust/Tauri
rewrite of the earlier Avalonia app, with the same features and the same `~/.dbclient` files.

## Features

- **Connections:** SQL Server (TCP, named instances, custom port), Azure SQL with your `az login`
  session, MySQL / MariaDB, SQLite files. SSH tunnelling with a password or private key, host keys
  pinned on first use. Credentials encrypted at rest. Per-connection timeouts.
- **Workspace:** a tab per connection (color-coded) and, inside it, a separate set of query tabs
  **per database**; each tab carries a badge in its database's color. Drag to reorder, rename
  inline, close others / to the right / to the left. Everything is restored on launch.
- **Editor:** CodeMirror with SQL highlighting and **context-aware IntelliSense** (tables in scope,
  columns of those tables, aliases, JOIN conditions, dialect keywords). Run the selection or the
  whole editor, format, explain, toggle comments, cancel a running query.
- **Schema tree:** databases, schemas, tables, views, procedures, columns (type, PK, nullability).
  Filter bar with AND/OR syntax (`customer order|client name`). Right-click for SELECT / INSERT /
  UPDATE / DELETE / CREATE / DROP scripts; double-click a table for `SELECT TOP(100)`.
- **Results:** virtualized, sortable grid; **edit cells** and apply a reviewed UPDATE script
  (primary keys detected); multiple result sets; in-grid filter; `NULL` shown distinctly; copy
  cell / rows / all / as INSERT; export CSV and JSON; row limit with a truncation banner; server
  messages; errors with line numbers.
- **History:** per-connection panel and a Query → History menu with search.
- **DACPAC:** export and import through SqlPackage when it's installed; rename and delete
  databases from the tree.
- **Themes:** Dark, Dracula and Light, switched at runtime (Ctrl+T).

## Install

Download `dbclient-Setup-<version>.exe` (or the portable zip) from
[Releases](https://github.com/timothydodd/dbclient/releases). It installs per user with no admin
prompt. Release builds are code-signed; check downloads against `SHA256SUMS.txt`.

Needs Windows 10 1809 or later. The installer adds the Microsoft Edge WebView2 runtime if it's
missing.

### Azure SQL

Sign in with the Azure CLI, then pick **Authentication: Azure (use az login)** in the connection
dialog:

```bash
az login
az account set --subscription "<your-sub>"
```

### DACPAC export / import

Install SqlPackage once (`dotnet tool install -g microsoft.sqlpackage`). The database node's
right-click menu then offers Export DACPAC and Import DACPAC.

## Keyboard shortcuts

| Shortcut | Action |
|---|---|
| Ctrl+N / Ctrl+W | New / close query tab |
| Ctrl+Tab / Ctrl+Shift+Tab | Next / previous query tab |
| Ctrl+O / Ctrl+S | Import / export a .sql file |
| F5 / Ctrl+Enter | Execute (selection or whole editor) |
| Esc | Cancel running query |
| Ctrl+E | Explain query plan |
| Ctrl+Shift+F | Format SQL |
| Ctrl+/ | Toggle line comment |
| Ctrl+F | Find in the focused pane |
| Ctrl+= / Ctrl+- / Ctrl+0 | Editor zoom |
| Alt+Z | Word wrap |
| Ctrl+Shift+E / Ctrl+Shift+R | Schema filter / results filter |
| Ctrl+L / Ctrl+H | Connection panel / history panel |
| Ctrl+T | Cycle theme |
| F1 | Keyboard shortcuts |

## Documentation

| | |
| --- | --- |
| [Data and privacy](docs/data-and-privacy.md) | What's stored in `~/.dbclient` |
| [Development](docs/development.md) | Building and running from source |
| [Releasing](docs/releasing.md) | Release pipeline and code signing (maintainers) |

## License

dbclient is released under the [MIT License](LICENSE). It's built on open-source components that
keep their own licenses; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). Every release
includes the full license texts (`THIRD_PARTY_LICENSES.txt`).
