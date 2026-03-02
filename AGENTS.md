# Repository Guidelines

## Project Structure & Module Organization
- `BatCave/`: WinUI host app (XAML views, view models, app bootstrap, publish profiles, assets).
- `BatCave.Core/`: runtime/domain logic (collectors, telemetry pipeline, sorting, persistence, policies).
- `BatCave.Core.Tests/`: unit tests for core runtime and services.
- `BatCave.Tests/`: UI/view-model-focused tests for host-side behavior.
- `docs/`: currently absent; if reintroduced, use it for spec/traceability artifacts (`requirements.md`, `design.md`, `tasks.md`, `validation.md`, `blueprint.md`).
- `scripts/`: repeatable local workflows (`run-dev.ps1`, `run-benchmark.ps1`, `validate-winui.ps1`).

## Build, Test, and Development Commands
- `dotnet build BatCave.slnx`: build all projects.
- `dotnet test BatCave.slnx`: run all tests.
- `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-dev.ps1 -Platform x64`: build (unless `-NoBuild`) and run the WinUI app.
- `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-benchmark.ps1 -Platform x64 -Ticks 120 -SleepMs 1000`: run CLI benchmark mode.
- `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/validate-winui.ps1`: required validation gate before handoff (build + tests).

## Coding Style & Naming Conventions
- Language: C# (`net8.0-windows10.0.19041.0`) with nullable reference types enabled.
- Follow existing code style: 4-space indentation, file-scoped namespaces, `PascalCase` for types/members, `_camelCase` for private fields.
- Keep UI behavior in view models/services; keep code-behind limited to UI wiring.
- Prefer focused, minimal edits; keep domain logic in `BatCave.Core`.

## Testing Guidelines
- Frameworks: xUnit (`BatCave.Core.Tests`, `BatCave.Tests`) with coverlet collector present in `BatCave.Tests`.
- Test files should end with `Tests.cs`; test method names should describe behavior (example: `DegradeMode_TransitionsByOverBudgetAndRecoveryStreaks`).
- Run `dotnet test BatCave.slnx` locally before opening a PR.

## Commit & Pull Request Guidelines
- Follow observed commit prefixes: `feat:`, `test:`, `docs:`, and task-oriented entries like `Task 7: ...`.
- Keep commit subjects imperative and scoped to one change.
- PRs should include: summary, linked task/issue, validation evidence (command(s) run), and screenshots/GIFs for UI changes.
- If the optional `docs/` tree exists and you update numbered work in `docs/tasks.md`, keep `docs/validation.md` in sync.

## Security & Configuration Notes
- Persistence is local-only under `%LOCALAPPDATA%\BatCaveMonitor`.
- Do not introduce outbound network dependencies or telemetry uploads.
