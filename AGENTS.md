# Repository Guidelines

## Project Structure & Module Organization
- `src/BatCave.App/`: production Rust + Tauri + Svelte desktop app, including frontend UI, Rust runtime store, native Windows/Linux telemetry collectors, app icon, helper modes, benchmark CLI, and platform packaging config.
- `scripts/`: repeatable local workflows (`run-dev.ps1`, `run-benchmark.ps1`, `capture-benchmark-baseline.ps1`, `validate-tauri.ps1`).
- `artifacts/`: generated benchmark, screenshot, and comparison outputs; treat as disposable generated output.

## Build, Test, and Development Commands
- `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-dev.ps1`: build the Svelte frontend and launch the Tauri desktop app.
- `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-dev.ps1 -WebOnly`: run the browser dev server with fixture telemetry.
- `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/validate-tauri.ps1`: run frontend build/type/lint/format checks, Rust fmt/check/test, and the Tauri bundle.
- `bash scripts/install-linux-deps.sh`: install Ubuntu/Debian native Tauri prerequisites.
- `bash scripts/run-dev.sh` and `bash scripts/run-dev.sh --web-only`: Linux app/web launch workflows.
- `bash scripts/validate-tauri.sh`: Linux validation equivalent for frontend build/type/lint/format checks, Rust fmt/check/test, and the Tauri bundle.
- `cargo test --manifest-path src/BatCave.App/src-tauri/Cargo.toml`: run Rust runtime, contract, collector, helper, and benchmark tests.
- `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-benchmark.ps1 -BenchmarkHost core -Platform x64 -Ticks 120 -SleepMs 1000`: run benchmark gates through the Rust runtime host.
- `bash scripts/run-benchmark.sh --benchmark-host core --ticks 120 --sleep-ms 1000`: Linux benchmark gate.
- `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/capture-benchmark-baseline.ps1 -BenchmarkHost core -Platform x64`: capture baseline summaries and artifacts under `artifacts/benchmarks`.

## Coding Style & Naming Conventions
- Tauri backend code uses Rust; prefer existing module boundaries in `src-tauri/src`.
- Frontend code uses TypeScript/Svelte; prefer the existing component, CSS variable, and fixture patterns.
- Keep long-lived runtime state in the Rust store. Keep browser fixture behavior deterministic and local to frontend development.
- Preserve snake_case JSON payload contracts for runtime, benchmark, helper, and validation surfaces.

## Testing Guidelines
- Use Rust unit tests for contract JSON, persistence behavior, runtime store shaping, collector math, helper argument parsing, and benchmark parsing.
- Run `scripts/validate-tauri.ps1` for app or runtime changes.
- For screenshot-visible UI work, capture fresh browser/Tauri evidence across desktop and mobile-sized viewports and use the two-lens review loop requested by the user.

## Commit & Pull Request Guidelines
- Follow observed commit prefixes: `feat:`, `fix:`, `test:`, `docs:`, and task-oriented entries like `Task 7: ...`.
- Keep commit subjects imperative and scoped to one change.
- PRs should include: summary, linked task/issue, validation evidence, and screenshots/GIFs for UI changes.

## Security & Configuration Notes
- Persistence, warm cache, helper snapshots, and logs are local-only under `%LOCALAPPDATA%\BatCaveMonitor` on Windows and `$XDG_DATA_HOME/BatCaveMonitor` or `~/.local/share/BatCaveMonitor` on Linux.
- Do not introduce outbound network dependencies, analytics, telemetry uploads, or remote logging.
- Admin mode must use local helper data only and must continue to fall back to standard access if elevation is denied or unavailable.
