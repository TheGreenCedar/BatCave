#[cfg(all(target_os = "macos", not(target_arch = "aarch64")))]
compile_error!("BatCave supports macOS on Apple Silicon only.");

mod app_icon;
mod atomic_json;
mod background_persistence;
mod benchmark;
mod cli_args;
mod collector_engine;
#[cfg_attr(not(test), allow(dead_code))]
mod collector_service;
mod contracts;
mod desktop_probe;
#[cfg(any(windows, test))]
mod legacy_helper_migration;
#[cfg(any(target_os = "linux", test))]
mod linux_network;
#[cfg(any(target_os = "linux", test))]
mod linux_process;
#[cfg(any(target_os = "linux", test))]
mod linux_system;
#[cfg(target_os = "macos")]
mod macos_network;
#[cfg(target_os = "macos")]
mod macos_process;
#[cfg(target_os = "macos")]
mod macos_system;
mod narratives;
#[cfg(any(windows, target_os = "linux", target_os = "macos", test))]
mod network_attribution;
mod persistence;
mod persistence_proof;
mod process_icons;
mod protocol;
mod runtime_health;
mod runtime_provenance;
mod runtime_store;
mod runtime_ui_preferences;
mod telemetry;
#[cfg(test)]
mod updater_hostile_fixtures;
#[cfg(all(windows, feature = "private-windows-lifecycle-proof"))]
mod windows_lifecycle_proof;
#[cfg(any(test, feature = "private-windows-lifecycle-proof"))]
#[cfg_attr(not(feature = "private-windows-lifecycle-proof"), allow(dead_code))]
mod windows_lifecycle_proof_contract;
#[cfg(any(windows, test))]
mod windows_network;
#[cfg(any(windows, test))]
mod windows_pdh;
#[cfg(any(windows, test))]
mod windows_process;
#[cfg(any(windows, test))]
mod windows_system;
#[cfg(any(windows, test))]
mod windows_user_launch;
mod workload_history;
mod workload_identity;

use contracts::{ProcessFocusMode, RuntimeQuery, SortColumn, SortDirection};
use narratives::{
    NarrativeFactPacket, NarrativeGenerationResponse, NarrativeModelStatus, NarrativePreferences,
    NarrativeRequest, NarrativeState,
};
use protocol::{
    ProcessFocusModeV4, ProtocolEnvelope, RuntimeQueryInputV4, RuntimeUiPreferencesV4,
    SortColumnV4, SortDirectionV4,
};
use runtime_store::RuntimeState;
use std::collections::HashMap;
use tauri::{AppHandle, Manager};

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

#[cfg(all(windows, feature = "private-windows-lifecycle-proof"))]
pub fn run_windows_lifecycle_proof() -> i32 {
    windows_lifecycle_proof::run()
}

fn run_cli(args: &[String]) -> Option<i32> {
    persistence_proof::run_cli(args).or_else(|| benchmark::run_cli(args))
}

#[tauri::command(async)]
fn get_snapshot(state: tauri::State<'_, RuntimeState>) -> Result<ProtocolEnvelope, String> {
    protocol::encode_snapshot(state.snapshot()?)
}

#[tauri::command(async)]
fn get_workload_inspection(
    state: tauri::State<'_, RuntimeState>,
    stable_id: String,
    point_limit: u16,
) -> Result<workload_history::WorkloadInspection, String> {
    state.workload_inspection(&stable_id, point_limit)
}

#[tauri::command(async)]
fn acknowledge_workload_inspection(
    state: tauri::State<'_, RuntimeState>,
    response_token: String,
) -> Result<(), String> {
    state.acknowledge_workload_inspection(&response_token)
}

#[tauri::command]
fn desktop_probe_enabled(state: tauri::State<'_, desktop_probe::DesktopProbe>) -> bool {
    state.enabled()
}

#[tauri::command]
fn record_desktop_probe(
    state: tauri::State<'_, desktop_probe::DesktopProbe>,
    narrative: tauri::State<'_, NarrativeState>,
    observation: desktop_probe::Observation,
) -> Result<(), String> {
    state.record(observation, narrative.preferences().enhanced_narratives)
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
    query: RuntimeQueryInputV4,
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

fn runtime_query(query: RuntimeQueryInputV4) -> Result<RuntimeQuery, String> {
    Ok(RuntimeQuery {
        filter_text: query.filter_text,
        focus_mode: match query.focus_mode {
            ProcessFocusModeV4::All => ProcessFocusMode::All,
            ProcessFocusModeV4::Attention => ProcessFocusMode::Attention,
            ProcessFocusModeV4::Io => ProcessFocusMode::Io,
        },
        sort_column: match query.sort_column {
            SortColumnV4::Attention => SortColumn::Attention,
            SortColumnV4::Name => SortColumn::Name,
            SortColumnV4::Pid => SortColumn::Pid,
            SortColumnV4::CpuPct => SortColumn::CpuPct,
            SortColumnV4::MemoryBytes => SortColumn::MemoryBytes,
            SortColumnV4::IoBps => SortColumn::IoBps,
            SortColumnV4::NetworkBps => SortColumn::NetworkBps,
            SortColumnV4::Threads => SortColumn::Threads,
            SortColumnV4::Handles => SortColumn::Handles,
            SortColumnV4::StartTimeMs => SortColumn::StartTimeMs,
        },
        sort_direction: match query.sort_direction {
            SortDirectionV4::Asc => SortDirection::Asc,
            SortDirectionV4::Desc => SortDirection::Desc,
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
    preferences: RuntimeUiPreferencesV4,
) -> Result<ProtocolEnvelope, String> {
    let preferences = runtime_ui_preferences::parse(preferences)?;
    protocol::encode_snapshot(state.set_ui_preferences(preferences)?)
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

#[tauri::command(async)]
fn sync_app_appearance(app: AppHandle, theme: String) -> Result<(), String> {
    app_icon::sync(&app, &theme)
}

#[tauri::command(async)]
fn get_narrative_preferences(
    state: tauri::State<'_, NarrativeState>,
) -> Result<NarrativePreferences, String> {
    Ok(state.preferences())
}

#[tauri::command(async)]
fn set_enhanced_narratives(
    state: tauri::State<'_, NarrativeState>,
    enabled: bool,
) -> Result<NarrativePreferences, String> {
    state.set_enhanced_narratives(enabled)
}

#[tauri::command(async)]
fn get_narrative_capability(
    state: tauri::State<'_, NarrativeState>,
) -> Result<NarrativeModelStatus, String> {
    Ok(state.capability())
}

#[tauri::command(async)]
fn get_narrative_fact_digest(
    state: tauri::State<'_, NarrativeState>,
    facts: NarrativeFactPacket,
) -> Result<String, String> {
    state.fact_digest(&facts)
}

#[tauri::command]
async fn generate_narrative(
    state: tauri::State<'_, NarrativeState>,
    request: NarrativeRequest,
    facts: NarrativeFactPacket,
) -> Result<NarrativeGenerationResponse, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.generate(request, facts))
        .await
        .map_err(|_| "narrative_generation_task_failed".to_string())?
}

#[tauri::command(async)]
fn cancel_narrative_generation(state: tauri::State<'_, NarrativeState>) -> Result<(), String> {
    state.cancel_generation();
    Ok(())
}

#[tauri::command]
async fn download_narrative_model(
    state: tauri::State<'_, NarrativeState>,
) -> Result<NarrativeModelStatus, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.download_model())
        .await
        .map_err(|_| "narrative_model_download_task_failed".to_string())
}

#[tauri::command(async)]
fn cancel_narrative_model_download(
    state: tauri::State<'_, NarrativeState>,
) -> Result<NarrativeModelStatus, String> {
    Ok(state.cancel_model_download())
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
            #[cfg(windows)]
            std::thread::Builder::new()
                .name("batcave-user-launch".into())
                .spawn(|| {
                    if let Err(error) = windows_user_launch::ensure_current_user_start_entry() {
                        eprintln!("current_user_launch_entry_failed:{error}");
                    }
                })?;
            let state = RuntimeState::new().map_err(std::io::Error::other)?;
            state.start();
            if !app.manage(state) {
                return Err(std::io::Error::other("runtime_state_already_managed").into());
            }
            let narrative_resource_dir = app.path().resource_dir().ok();
            let narrative = NarrativeState::new(narrative_resource_dir);
            app.manage(
                desktop_probe::DesktopProbe::from_env(&narrative).map_err(std::io::Error::other)?,
            );
            if !app.manage(narrative) {
                return Err(std::io::Error::other("narrative_state_already_managed").into());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            get_workload_inspection,
            acknowledge_workload_inspection,
            desktop_probe_enabled,
            record_desktop_probe,
            refresh_now,
            pause_runtime,
            resume_runtime,
            set_process_query,
            set_sample_interval,
            set_ui_preferences,
            get_process_icons,
            sync_app_appearance,
            get_narrative_preferences,
            set_enhanced_narratives,
            get_narrative_capability,
            get_narrative_fact_digest,
            generate_narrative,
            cancel_narrative_generation,
            download_narrative_model,
            cancel_narrative_model_download
        ])
        .build(tauri::generate_context!())
        .map_err(|error| format!("desktop_runtime_build_failed:{error}"))?;
    app.run(|app_handle, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            app_handle.state::<NarrativeState>().shutdown();
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
        let wire: RuntimeQueryInputV4 = serde_json::from_value(serde_json::json!({
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
}
