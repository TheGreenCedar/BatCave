mod atomic_json;
mod benchmark;
mod cli_args;
mod contracts;
#[cfg(test)]
#[allow(dead_code)]
mod elevation;
#[cfg(any(target_os = "linux", test))]
mod linux_network;
#[cfg(any(target_os = "linux", test))]
mod linux_process;
#[cfg(any(target_os = "linux", test))]
mod linux_system;
#[cfg(target_os = "macos")]
mod macos_process;
#[cfg(target_os = "macos")]
mod macos_system;
#[cfg(any(windows, target_os = "linux", test))]
mod network_attribution;
mod process_icons;
mod runtime_store;
mod telemetry;
#[cfg(any(windows, test))]
mod windows_network;
#[cfg(any(windows, test))]
mod windows_pdh;
#[cfg(any(windows, test))]
mod windows_process;
#[cfg(any(windows, test))]
mod windows_system;

use contracts::{RuntimeQuery, RuntimeSnapshot};
use runtime_store::RuntimeState;
use std::collections::HashMap;
use tauri::Manager;

pub fn run_cli_from_env() -> Option<i32> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    benchmark::run_cli(&args)
}

#[tauri::command]
fn get_snapshot(state: tauri::State<'_, RuntimeState>) -> Result<RuntimeSnapshot, String> {
    state.snapshot()
}

#[tauri::command]
fn refresh_now(state: tauri::State<'_, RuntimeState>) -> Result<RuntimeSnapshot, String> {
    state.refresh_now()
}

#[tauri::command]
fn pause_runtime(state: tauri::State<'_, RuntimeState>) -> Result<RuntimeSnapshot, String> {
    state.pause()
}

#[tauri::command]
fn resume_runtime(state: tauri::State<'_, RuntimeState>) -> Result<RuntimeSnapshot, String> {
    state.resume()
}

#[tauri::command]
fn set_process_query(
    state: tauri::State<'_, RuntimeState>,
    query: RuntimeQuery,
) -> Result<RuntimeSnapshot, String> {
    state.set_query(query)
}

#[tauri::command]
fn set_sample_interval(
    state: tauri::State<'_, RuntimeState>,
    sample_interval_ms: u32,
) -> Result<RuntimeSnapshot, String> {
    state.set_sample_interval(sample_interval_ms)
}

#[tauri::command]
fn get_process_icons(
    state: tauri::State<'_, RuntimeState>,
    exes: Vec<String>,
) -> Result<HashMap<String, Option<String>>, String> {
    if exes.len() > 120 {
        return Err("process_icon_batch_too_large".to_string());
    }
    exes.into_iter()
        .map(|exe| {
            validate_process_icon_request(&exe, |candidate| state.has_process_exe(candidate))?;
            Ok((exe.clone(), process_icons::icon_data_url(&exe)?))
        })
        .collect()
}

fn validate_process_icon_request(
    exe: &str,
    mut has_process_exe: impl FnMut(&str) -> Result<bool, String>,
) -> Result<(), String> {
    let exe = exe.trim();
    if exe.is_empty() {
        return Ok(());
    }
    if exe.starts_with(r"\\") || exe.starts_with("//") {
        return Err("process_icon_unc_path_rejected".to_string());
    }
    if !has_process_exe(exe)? {
        return Err("process_icon_untrusted_exe".to_string());
    }
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let state = RuntimeState::new();
            state.start();
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            refresh_now,
            pause_runtime,
            resume_runtime,
            set_process_query,
            set_sample_interval,
            get_process_icons
        ])
        .run(tauri::generate_context!())
        .expect("error while running BatCave Monitor");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_icon_request_rejects_unc_paths() {
        assert_eq!(
            validate_process_icon_request(r"\\server\share\app.exe", |_| Ok(true)),
            Err("process_icon_unc_path_rejected".to_string())
        );
        assert_eq!(
            validate_process_icon_request("//server/share/app.exe", |_| Ok(true)),
            Err("process_icon_unc_path_rejected".to_string())
        );
    }

    #[test]
    fn process_icon_request_rejects_unseen_paths() {
        assert_eq!(
            validate_process_icon_request(r"C:\Windows\System32\notepad.exe", |_| Ok(false)),
            Err("process_icon_untrusted_exe".to_string())
        );
    }

    #[test]
    fn process_icon_request_allows_seen_paths_and_empty_input() {
        assert_eq!(
            validate_process_icon_request(r"C:\Windows\explorer.exe", |_| Ok(true)),
            Ok(())
        );
        assert_eq!(validate_process_icon_request("", |_| Ok(false)), Ok(()));
    }
}
