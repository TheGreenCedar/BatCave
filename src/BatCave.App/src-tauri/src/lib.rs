mod contracts;
mod telemetry;

use contracts::RuntimeSnapshot;
use telemetry::TelemetryState;

#[tauri::command]
fn get_snapshot(state: tauri::State<'_, TelemetryState>) -> Result<RuntimeSnapshot, String> {
    telemetry::collect_snapshot(&state).map_err(|error| error.to_string())
}

pub fn run() {
    tauri::Builder::default()
        .manage(TelemetryState::new())
        .invoke_handler(tauri::generate_handler![get_snapshot])
        .run(tauri::generate_context!())
        .expect("error while running BatCave Monitor");
}
