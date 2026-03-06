# Repository Guidelines

## Project Structure & Module Organization
- `BatCave/`: WinUI 3 host app, XAML controls, view models, styling, assets, publish profiles, and WinUI-side CLI surfaces used by validation scripts.
- `BatCave.Core/`: shared runtime/domain/CLI/persistence layer (collectors, telemetry pipeline, runtime loop, sort/state engines, launch policy, JSON persistence).
- `BatCave.Bench/`: headless benchmark host for core runtime perf runs and strict gate comparisons.
- `BatCave.Core.Tests/`: xUnit coverage for runtime, persistence, benchmark, and PowerShell script regression behavior.
- `BatCave.Tests/`: xUnit coverage for source-linked host logic, layout/chart math, hosting services, and XAML accessibility contracts.
- `scripts/`: repeatable local workflows (`run-dev.ps1`, `run-benchmark.ps1`, `capture-benchmark-baseline.ps1`, `validate-winui.ps1`).
- `artifacts/`: generated benchmark artifacts and comparison outputs; treat as disposable generated output.

## Build, Test, and Development Commands
- `dotnet build BatCave.slnx`: build all projects, including `BatCave.Bench`.
- `dotnet test BatCave.slnx`: run unit, UI-adjacent, and script regression tests.
- `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-dev.ps1 -Platform x64`: build (unless `-NoBuild`) and run the WinUI app.
- `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-benchmark.ps1 -BenchmarkHost core -Platform x64 -Ticks 120 -SleepMs 1000`: run benchmark gates through the headless core host; use `-BenchmarkHost winui` for the WinUI-driven benchmark path.
- `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/capture-benchmark-baseline.ps1 -BenchmarkHost core -Platform x64`: capture baseline summaries and artifacts under `artifacts/benchmarks`.
- `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/validate-winui.ps1 -Platform ARM64`: handoff gate; builds the WinUI project, runs solution tests, verifies `--print-runtime-health`, and performs a launch smoke by default. Add `-RunPerformanceGate` with baseline args for strict perf validation.

## Coding Style & Naming Conventions
- Language: C# (`net10.0-windows10.0.19041.0`) with nullable reference types enabled.
- Follow existing code style: 4-space indentation, file-scoped namespaces, `PascalCase` for types/members, `_camelCase` for private fields.
- Keep UI behavior in view models/services; keep code-behind limited to UI wiring.
- Prefer focused, minimal edits; keep shared runtime/CLI logic in `BatCave.Core` and WinUI-only orchestration in `BatCave`.

## Testing Guidelines
- Frameworks: xUnit in both test projects; `coverlet.collector` is currently configured in `BatCave.Tests` only.
- `BatCave.Core.Tests` covers core runtime, persistence, CLI/benchmark behavior, and PowerShell script regressions; `BatCave.Tests` covers source-linked host logic and XAML-facing contracts rather than packaged UI automation.
- Test files should end with `Tests.cs`; test method names should describe behavior (example: `DegradeMode_TransitionsByOverBudgetAndRecoveryStreaks`).
- When changing PowerShell workflows, update or add coverage in `BatCave.Core.Tests/Scripts`.
- When changing shell XAML semantics, theme resources, or accessibility labels, update or add coverage in `BatCave.Tests/Ui`.
- Run `dotnet test BatCave.slnx` locally before opening a PR, and use `scripts/validate-winui.ps1` for UI/runtime-affecting changes.

## Commit & Pull Request Guidelines
- Follow observed commit prefixes: `feat:`, `fix:`, `test:`, `docs:`, and task-oriented entries like `Task 7: ...`.
- Keep commit subjects imperative and scoped to one change.
- PRs should include: summary, linked task/issue, validation evidence (command(s) run), and screenshots/GIFs for UI changes.

## Security & Configuration Notes
- Persistence, warm cache, and logs are local-only under `%LOCALAPPDATA%\BatCaveMonitor`.
- Keep Serilog rolling file logs local under `%LOCALAPPDATA%\BatCaveMonitor\logs`; do not introduce outbound network dependencies or telemetry uploads.
- Preserve CLI/script-facing contracts when possible; validation and benchmark flows depend on the current argument names, snake_case JSON payloads, and elevated-helper path/token handoff.
