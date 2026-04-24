# Repository Guidelines

## Project Structure & Module Organization
- `src/BatCave.App/`: production Rust + Tauri + Svelte desktop app, including frontend UI, native telemetry collector, app icon, and NSIS packaging config.
- `src/BatCave.Runtime/`: shared runtime/domain/CLI/persistence layer (collectors, immutable contracts, single-writer runtime store, reducer, launch policy, JSON persistence).
- `src/BatCave.Bench/`: headless benchmark host for core runtime perf runs and strict gate comparisons.
- `tests/BatCave.Runtime.Tests/`: xUnit coverage for runtime contracts, persistence recovery, benchmark contracts, reducer behavior, JSON shape, and bounded event coalescing.
- `scripts/`: repeatable local workflows (`run-dev.ps1`, `run-benchmark.ps1`, `capture-benchmark-baseline.ps1`, `validate-tauri.ps1`).
- `artifacts/`: generated benchmark, screenshot, and comparison outputs; treat as disposable generated output.

## Build, Test, and Development Commands
- `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-dev.ps1`: build the Svelte frontend and launch the Tauri desktop app.
- `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-dev.ps1 -WebOnly`: run the browser dev server with fixture telemetry.
- `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/validate-tauri.ps1`: run frontend build/type/lint/format checks, Rust fmt/check, Tauri bundle, and .NET tests.
- `dotnet build BatCave.slnx`: build runtime and benchmark projects.
- `dotnet test BatCave.slnx`: run runtime, script, persistence, reducer, and benchmark contract tests.
- `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-benchmark.ps1 -BenchmarkHost core -Platform x64 -Ticks 120 -SleepMs 1000`: run benchmark gates through the headless core host.
- `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/capture-benchmark-baseline.ps1 -BenchmarkHost core -Platform x64`: capture baseline summaries and artifacts under `artifacts/benchmarks`.

## Coding Style & Naming Conventions
- C# runtime code targets `net10.0-windows10.0.19041.0` with nullable reference types enabled.
- Follow existing C# style: 4-space indentation, file-scoped namespaces, `PascalCase` for types/members, `_camelCase` for private fields.
- Tauri app code uses TypeScript/Svelte and Rust; prefer the existing component, CSS variable, and collector patterns.
- Keep shared runtime/CLI logic in `src/BatCave.Runtime`; keep desktop UI and Tauri-native orchestration in `src/BatCave.App`.

## Testing Guidelines
- Framework: xUnit for .NET tests.
- `tests/BatCave.Runtime.Tests` covers core runtime, persistence, CLI/benchmark behavior, script behavior, and reducer behavior.
- Run `dotnet test BatCave.slnx` for runtime changes.
- Run `scripts/validate-tauri.ps1` for desktop app changes.
- For screenshot-visible UI work, capture fresh browser/Tauri evidence across desktop and mobile-sized viewports and use the two-lens review loop requested by the user.

## Commit & Pull Request Guidelines
- Follow observed commit prefixes: `feat:`, `fix:`, `test:`, `docs:`, and task-oriented entries like `Task 7: ...`.
- Keep commit subjects imperative and scoped to one change.
- PRs should include: summary, linked task/issue, validation evidence, and screenshots/GIFs for UI changes.

## Security & Configuration Notes
- Persistence, warm cache, and logs are local-only under `%LOCALAPPDATA%\BatCaveMonitor`.
- Do not introduce outbound network dependencies, analytics, telemetry uploads, or remote logging.
- Preserve snake_case JSON payload contracts for runtime, benchmark, and validation surfaces when possible.
