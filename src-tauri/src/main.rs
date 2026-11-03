// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "macos")]
    extend_path_for_gui_launch();
    dbclient_lib::run()
}

/// Apps opened from Finder/Dock get launchd's minimal PATH (`/usr/bin:/bin:/usr/sbin:/sbin`),
/// so tools installed by Homebrew or `dotnet tool` (`az`, `sqlpackage`) aren't found. Append the
/// usual install locations that exist and aren't already on PATH.
#[cfg(target_os = "macos")]
fn extend_path_for_gui_launch() {
    let current = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs: Vec<std::path::PathBuf> = std::env::split_paths(&current).collect();
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let mut candidates: Vec<std::path::PathBuf> =
        ["/opt/homebrew/bin", "/opt/homebrew/sbin", "/usr/local/bin", "/usr/local/sbin"]
            .iter()
            .map(std::path::PathBuf::from)
            .collect();
    if let Some(h) = home {
        candidates.push(h.join(".dotnet/tools"));
        candidates.push(h.join(".local/bin"));
    }
    for c in candidates {
        if c.is_dir() && !dirs.contains(&c) {
            dirs.push(c);
        }
    }
    if let Ok(joined) = std::env::join_paths(dirs) {
        std::env::set_var("PATH", joined);
    }
}
