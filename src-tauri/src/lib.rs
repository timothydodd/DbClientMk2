mod commands;

use std::sync::Arc;

use dbclient_data::HostKeyInfo;
use dbclient_engine::{EventSink, Workspace};
use dbclient_store::{logger, AppPaths, Store};
use tauri::{AppHandle, Emitter, Manager};

/// Forwards engine callbacks to the webview as global events.
struct TauriSink(AppHandle);

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct HostKeyEvent<'a> {
    request_id: &'a str,
    #[serde(flatten)]
    info: &'a HostKeyInfo,
}

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SqlPackageLine<'a> {
    job_id: &'a str,
    line: &'a str,
}

impl EventSink for TauriSink {
    fn host_key_prompt(&self, request_id: &str, info: &HostKeyInfo) {
        let _ = self.0.emit("ssh:host-key", HostKeyEvent { request_id, info });
    }
    fn sqlpackage_output(&self, job_id: &str, line: &str) {
        let _ = self.0.emit("sqlpackage:output", SqlPackageLine { job_id, line });
    }
}

pub struct AppState {
    pub workspace: Arc<Workspace>,
}

fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let paths = AppPaths::resolve();
    let _ = paths.ensure_directory();
    logger::init(paths.clone());
    logger::info(format!(
        "==== dbclient {} starting | OS: {} {} | Data dir: {} | PID: {}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        paths.root.display(),
        std::process::id()
    ));

    tauri::Builder::default()
        // Must be registered first: a second launch just focuses this instance.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main_window(app)))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .setup(move |app| {
            let store = Arc::new(Store::new(paths.clone()));
            let workspace = Workspace::new(store, Arc::new(TauriSink(app.handle().clone())), env!("CARGO_PKG_VERSION").to_string());
            app.manage(AppState { workspace });
            show_main_window(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_state,
            commands::save_app_state,
            commands::upsert_connection,
            commands::delete_connection,
            commands::load_connection_state,
            commands::save_connection_state,
            commands::open_connection,
            commands::close_connection,
            commands::reload_databases,
            commands::load_schema,
            commands::execute_query,
            commands::cancel_query,
            commands::get_completions,
            commands::completion_accepted,
            commands::get_hover,
            commands::get_diagnostics,
            commands::test_connection,
            commands::rename_database,
            commands::drop_database,
            commands::external_connection_string,
            commands::get_history,
            commands::add_history,
            commands::answer_host_key,
            commands::sqlpackage_path,
            commands::run_sqlpackage,
            commands::cancel_sqlpackage,
            commands::read_text_file,
            commands::write_text_file,
            commands::file_exists,
            commands::app_info,
            commands::format_sql,
            commands::generate_update_script,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                let ws = Arc::clone(&app.state::<AppState>().workspace);
                tauri::async_runtime::block_on(ws.close_all());
                logger::info("dbclient exited");
            }
        });
}
