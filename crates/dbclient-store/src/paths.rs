//! Single source of truth for the data directory (`~/.dbclient`, or `DBCLIENT_HOME`) and safe
//! (atomic, permission-restricted) file writes.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub root: PathBuf,
}

impl AppPaths {
    /// `DBCLIENT_HOME` when set (tests, portable installs), else `~/.dbclient`.
    pub fn resolve() -> Self {
        if let Ok(p) = std::env::var("DBCLIENT_HOME") {
            if !p.trim().is_empty() {
                return Self { root: PathBuf::from(p) };
            }
        }
        let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_else(|_| ".".into());
        Self { root: Path::new(&home).join(".dbclient") }
    }

    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn connections_dir(&self) -> PathBuf {
        self.root.join("connections")
    }
    pub fn state_file(&self) -> PathBuf {
        self.root.join("state.json")
    }
    pub fn history_file(&self) -> PathBuf {
        self.root.join("history.json")
    }
    pub fn log_file(&self) -> PathBuf {
        self.root.join("log.txt")
    }
    pub fn prev_log_file(&self) -> PathBuf {
        self.root.join("log.prev.txt")
    }
    pub fn salt_file(&self) -> PathBuf {
        self.root.join(".salt")
    }
    pub fn known_hosts_file(&self) -> PathBuf {
        self.root.join("known_hosts")
    }

    /// Creates the data directory (0700 on unix). Safe to call repeatedly.
    pub fn ensure_directory(&self) -> std::io::Result<()> {
        ensure_dir(&self.root)
    }
}

pub fn ensure_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    restrict_dir(dir);
    Ok(())
}

/// Writes to `path + ".tmp"` then renames over the target; parent directory is created.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        ensure_dir(dir)?;
    }
    let tmp = path.with_extension(match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => format!("{ext}.tmp"),
        None => "tmp".into(),
    });
    std::fs::write(&tmp, bytes)?;
    restrict_file(&tmp);
    std::fs::rename(&tmp, path)?;
    restrict_file(path);
    Ok(())
}

pub fn write_text_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    write_atomic(path, text.as_bytes())
}

/// chmod 0600 on unix; no-op on Windows.
pub fn restrict_file(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// chmod 0700 on unix; no-op on Windows.
pub fn restrict_dir(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
    }
    #[cfg(not(unix))]
    let _ = path;
}
