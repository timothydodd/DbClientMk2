# Data and privacy

Everything dbclient stores lives in `~/.dbclient` (override with the `DBCLIENT_HOME` environment
variable). Nothing is sent anywhere except to the database servers you connect to.

| File | Contents |
| --- | --- |
| `state.json` | Saved connections, which are open, layout and editor settings. Passwords, SSH passwords and key passphrases are encrypted (Windows DPAPI for the current user; AES-256-GCM with a per-install key elsewhere). `state.json.bak` is the previous good copy. |
| `connections/<id>.json` | A connection's query tabs (text, titles, order) grouped by database. |
| `history.json` | The last 100 executed queries (first 1000 characters each) with database and time. |
| `known_hosts` | SSH host keys you accepted, one per line (`host:port keytype base64`). `~/.ssh/known_hosts` is read too. |
| `log.txt` | Application log, rotated to `log.prev.txt` at 1 MB. Connection strings are redacted. |
| `.salt` | Random salt for the credential key on non-Windows platforms. |

The same files and formats as the earlier .NET version of dbclient, so existing saved connections
and tabs are picked up.

**DACPAC export/import** runs the SqlPackage tool as a child process with a connection string on
its command line; that string is visible in the OS process list while SqlPackage runs.
