mod atomic_json;
mod benchmark;
mod cli_args;
mod contracts;
mod elevation;
mod runtime_store;
mod telemetry;
mod windows_network;
mod windows_pdh;
mod windows_process;
mod windows_system;

use contracts::{RuntimeQuery, RuntimeSnapshot};
use runtime_store::RuntimeState;

pub fn run_cli_from_env() -> Option<i32> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    benchmark::run_cli(&args).or_else(|| elevation::run_cli(&args))
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
fn set_admin_mode(
    state: tauri::State<'_, RuntimeState>,
    enabled: bool,
) -> Result<RuntimeSnapshot, String> {
    state.set_admin_mode(enabled)
}

#[tauri::command]
fn set_process_query(
    state: tauri::State<'_, RuntimeState>,
    query: RuntimeQuery,
) -> Result<RuntimeSnapshot, String> {
    state.set_query(query)
}

pub fn run() {
    tauri::Builder::default()
        .manage(RuntimeState::new())
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            refresh_now,
            pause_runtime,
            resume_runtime,
            set_admin_mode,
            set_process_query
        ])
        .run(tauri::generate_context!())
        .expect("error while running BatCave Monitor");
}
