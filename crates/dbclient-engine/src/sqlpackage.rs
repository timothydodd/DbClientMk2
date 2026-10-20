//! Finds and runs the SqlPackage CLI (DACPAC extract / publish / script). Not bundled: the user
//! installs it (`dotnet tool install -g microsoft.sqlpackage`); output streams line by line.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tokio_util::sync::CancellationToken;

static CACHE: OnceLock<Mutex<Option<Option<PathBuf>>>> = OnceLock::new();

fn cache() -> &'static Mutex<Option<Option<PathBuf>>> {
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Full path of the executable, or `None` when it isn't installed. Cached after the first lookup.
pub fn executable_path() -> Option<PathBuf> {
    let mut c = cache().lock().unwrap();
    if let Some(cached) = c.as_ref() {
        return cached.clone();
    }
    let found = locate();
    match &found {
        Some(p) => dbclient_store::logger::info(format!("SqlPackage found at {}", p.display())),
        None => dbclient_store::logger::info("SqlPackage not found"),
    }
    *c = Some(found.clone());
    found
}

/// Forget the cached lookup (e.g. after the user installs the tool).
pub fn reset_cache() {
    *cache().lock().unwrap() = None;
}

fn locate() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) { &["sqlpackage.exe", "SqlPackage.exe"] } else { &["sqlpackage", "SqlPackage"] };
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            candidates.push(dir);
        }
    }
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
    candidates.push(Path::new(&home).join(".dotnet").join("tools"));
    if cfg!(windows) {
        for root in [std::env::var("ProgramFiles").ok(), std::env::var("ProgramFiles(x86)").ok()].into_iter().flatten() {
            let sql_root = Path::new(&root).join("Microsoft SQL Server");
            if let Ok(entries) = std::fs::read_dir(&sql_root) {
                let mut versions: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()).collect();
                versions.sort();
                versions.reverse();
                for v in versions {
                    candidates.push(v.join("DAC").join("bin"));
                }
            }
            candidates.push(Path::new(&root).join("SqlPackage"));
            candidates.push(Path::new(&root).join("sqlpackage"));
        }
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            candidates.push(Path::new(&local).join("Programs").join("sqlpackage"));
        }
    } else {
        candidates.push("/usr/local/bin".into());
        candidates.push("/usr/bin".into());
        candidates.push("/opt/sqlpackage".into());
        candidates.push(Path::new(&home).join("sqlpackage"));
    }
    for dir in candidates {
        for name in names {
            let full = dir.join(name);
            if full.is_file() {
                return Some(full);
            }
        }
    }
    None
}

/// Runs SqlPackage with `args`, handing each output line to `on_line`. Returns the exit code
/// (`None` when killed). Cancelling kills the process.
pub async fn run(args: &[String], cancel: CancellationToken, mut on_line: impl FnMut(String) + Send) -> Result<Option<i32>, String> {
    let exe = executable_path().ok_or_else(|| format!("SqlPackage was not found. {}", dbclient_core::sqlpackage::INSTALL_HINT))?;
    dbclient_store::logger::info(format!("SqlPackage {}", dbclient_core::sqlpackage::redact(args)));

    let mut cmd = tokio::process::Command::new(exe);
    cmd.args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().map_err(|e| format!("Couldn't start SqlPackage: {e}"))?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let stderr = child.stderr.take().ok_or("no stderr")?;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let pump = |reader: Box<dyn tokio::io::AsyncRead + Unpin + Send>, tx: tokio::sync::mpsc::UnboundedSender<String>| {
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let mut lines = tokio::io::BufReader::new(reader).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if tx.send(line).is_err() {
                    break;
                }
            }
        })
    };
    let out_task = pump(Box::new(stdout), tx.clone());
    let err_task = pump(Box::new(stderr), tx);

    // Stream lines as they arrive; stop on exit or cancel (kill_on_drop ends a cancelled process).
    let mut wait = Box::pin(child.wait());
    let status: Option<std::process::ExitStatus> = loop {
        tokio::select! {
            _ = cancel.cancelled() => break None,
            Some(line) = rx.recv() => on_line(line),
            s = &mut wait => break Some(s.map_err(|e| e.to_string())?),
        }
    };
    drop(wait);
    if status.is_none() {
        let _ = child.kill().await;
    }
    let _ = tokio::join!(out_task, err_task);
    while let Ok(line) = rx.try_recv() {
        on_line(line);
    }
    Ok(status.and_then(|s| s.code()))
}
