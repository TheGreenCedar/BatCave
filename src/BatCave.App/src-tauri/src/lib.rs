mod atomic_json;
mod benchmark;
mod cli_args;
mod collector_engine;
#[cfg_attr(not(test), allow(dead_code))]
mod collector_service;
mod contracts;
#[cfg(any(windows, test))]
mod legacy_helper_migration;
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
mod persistence;
mod persistence_proof;
mod process_icons;
mod protocol;
mod runtime_provenance;
mod runtime_store;
mod telemetry;
#[cfg(test)]
mod updater_hostile_fixtures;
#[cfg(any(windows, test))]
mod windows_network;
#[cfg(any(windows, test))]
mod windows_pdh;
#[cfg(any(windows, test))]
mod windows_process;
#[cfg(any(windows, test))]
mod windows_system;

use contracts::{ProcessFocusMode, RuntimeQuery, RuntimeUiPreferences, SortColumn, SortDirection};
use protocol::{
    ProcessFocusModeV3, ProtocolEnvelope, RuntimeQueryInputV3, RuntimeUiPreferencesV3,
    SortColumnV3, SortDirectionV3,
};
use runtime_store::RuntimeState;
use std::collections::HashMap;
use tauri::Manager;

pub fn run_cli_from_env() -> Option<i32> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    run_cli(&args)
}

#[cfg(windows)]
pub fn run_collector_service() -> i32 {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    collector_service::windows_provisioner::run_cli(&args)
        .unwrap_or_else(collector_service::windows_service::run)
}

fn run_cli(args: &[String]) -> Option<i32> {
    persistence_proof::run_cli(args).or_else(|| benchmark::run_cli(args))
}

#[tauri::command(async)]
fn get_snapshot(state: tauri::State<'_, RuntimeState>) -> Result<ProtocolEnvelope, String> {
    protocol::encode_snapshot(state.snapshot()?)
}

#[tauri::command(async)]
fn refresh_now(state: tauri::State<'_, RuntimeState>) -> Result<ProtocolEnvelope, String> {
    protocol::encode_snapshot(state.refresh_now()?)
}

#[tauri::command(async)]
fn pause_runtime(state: tauri::State<'_, RuntimeState>) -> Result<ProtocolEnvelope, String> {
    protocol::encode_snapshot(state.pause()?)
}

#[tauri::command(async)]
fn resume_runtime(state: tauri::State<'_, RuntimeState>) -> Result<ProtocolEnvelope, String> {
    protocol::encode_snapshot(state.resume()?)
}

#[tauri::command(async)]
fn set_process_query(
    state: tauri::State<'_, RuntimeState>,
    query: RuntimeQueryInputV3,
    persist: Option<bool>,
) -> Result<ProtocolEnvelope, String> {
    let query = runtime_query(query)?;
    protocol::encode_snapshot(if query_should_persist(persist) {
        state.set_query(query)?
    } else {
        state.set_query_runtime_only(query)?
    })
}

fn query_should_persist(persist: Option<bool>) -> bool {
    persist.unwrap_or(true)
}

fn runtime_query(query: RuntimeQueryInputV3) -> Result<RuntimeQuery, String> {
    Ok(RuntimeQuery {
        filter_text: query.filter_text,
        focus_mode: match query.focus_mode {
            ProcessFocusModeV3::All => ProcessFocusMode::All,
            ProcessFocusModeV3::Attention => ProcessFocusMode::Attention,
            ProcessFocusModeV3::Io => ProcessFocusMode::Io,
        },
        sort_column: match query.sort_column {
            SortColumnV3::Attention => SortColumn::Attention,
            SortColumnV3::Name => SortColumn::Name,
            SortColumnV3::Pid => SortColumn::Pid,
            SortColumnV3::CpuPct => SortColumn::CpuPct,
            SortColumnV3::MemoryBytes => SortColumn::MemoryBytes,
            SortColumnV3::IoBps => SortColumn::IoBps,
            SortColumnV3::NetworkBps => SortColumn::NetworkBps,
            SortColumnV3::Threads => SortColumn::Threads,
            SortColumnV3::Handles => SortColumn::Handles,
            SortColumnV3::StartTimeMs => SortColumn::StartTimeMs,
        },
        sort_direction: match query.sort_direction {
            SortDirectionV3::Asc => SortDirection::Asc,
            SortDirectionV3::Desc => SortDirection::Desc,
        },
        limit: usize::try_from(query.limit).map_err(|_| "protocol_query_limit_out_of_range")?,
    })
}

#[tauri::command(async)]
fn set_sample_interval(
    state: tauri::State<'_, RuntimeState>,
    sample_interval_ms: u32,
) -> Result<ProtocolEnvelope, String> {
    protocol::encode_snapshot(state.set_sample_interval(sample_interval_ms)?)
}

#[tauri::command(async)]
fn set_ui_preferences(
    state: tauri::State<'_, RuntimeState>,
    preferences: RuntimeUiPreferencesV3,
) -> Result<ProtocolEnvelope, String> {
    let preferences = runtime_ui_preferences(preferences)?;
    protocol::encode_snapshot(state.set_ui_preferences(preferences)?)
}

fn runtime_ui_preferences(
    preferences: RuntimeUiPreferencesV3,
) -> Result<RuntimeUiPreferences, String> {
    if !matches!(
        preferences.theme.as_str(),
        "system" | "cave" | "aurora" | "ember" | "daylight"
    ) {
        return Err("runtime_ui_theme_invalid".to_string());
    }
    if !matches!(preferences.history_point_limit, 30 | 72 | 180 | 360) {
        return Err("runtime_history_point_limit_invalid".to_string());
    }
    Ok(RuntimeUiPreferences {
        theme: preferences.theme,
        history_point_limit: preferences.history_point_limit,
    })
}

#[tauri::command(async)]
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

pub fn run() -> Result<(), String> {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let state = RuntimeState::new().map_err(std::io::Error::other)?;
            state.start();
            if !app.manage(state) {
                return Err(std::io::Error::other("runtime_state_already_managed").into());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            refresh_now,
            pause_runtime,
            resume_runtime,
            set_process_query,
            set_sample_interval,
            set_ui_preferences,
            get_process_icons
        ])
        .build(tauri::generate_context!())
        .map_err(|error| format!("desktop_runtime_build_failed:{error}"))?;
    app.run(|app_handle, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            if let Err(error) = app_handle.state::<RuntimeState>().shutdown() {
                eprintln!("runtime_shutdown_failed:{error}");
            }
        }
    });
    Ok(())
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

    #[test]
    fn generated_query_input_converts_at_the_command_boundary() {
        let wire: RuntimeQueryInputV3 = serde_json::from_value(serde_json::json!({
            "filter_text": "browser",
            "focus_mode": "io",
            "sort_column": "network_bps",
            "sort_direction": "asc",
            "limit": 25
        }))
        .expect("generated query input accepts the v3 wire shape");
        let query = runtime_query(wire).expect("v3 query converts to the runtime query");

        assert_eq!(query.filter_text, "browser");
        assert!(matches!(query.focus_mode, ProcessFocusMode::Io));
        assert!(matches!(query.sort_column, SortColumn::NetworkBps));
        assert!(matches!(query.sort_direction, SortDirection::Asc));
        assert_eq!(query.limit, 25);
    }

    #[test]
    fn missing_query_persistence_flag_preserves_the_legacy_durable_behavior() {
        assert!(query_should_persist(None));
        assert!(query_should_persist(Some(true)));
        assert!(!query_should_persist(Some(false)));
    }

    #[test]
    fn ui_preferences_validate_at_the_command_boundary() {
        let preferences = runtime_ui_preferences(RuntimeUiPreferencesV3 {
            theme: "ember".to_string(),
            history_point_limit: 180,
        })
        .expect("supported preferences convert");
        assert_eq!(preferences.theme, "ember");
        assert_eq!(preferences.history_point_limit, 180);

        assert_eq!(
            runtime_ui_preferences(RuntimeUiPreferencesV3 {
                theme: "remote-theme".to_string(),
                history_point_limit: 180,
            }),
            Err("runtime_ui_theme_invalid".to_string())
        );
        assert_eq!(
            runtime_ui_preferences(RuntimeUiPreferencesV3 {
                theme: "cave".to_string(),
                history_point_limit: 10_000,
            }),
            Err("runtime_history_point_limit_invalid".to_string())
        );
    }
}
