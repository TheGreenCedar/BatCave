# AGENTS.md

## Purpose
Shared working rules for agents contributing to `BatCave` (WinUI + .NET port with v1 parity expectations).

## Project Shape
- WinUI host: `BatCave/`
- Runtime/business logic: `BatCave.Core/`
- Tests: `BatCave.Core.Tests/`, `BatCave.Tests/`
- Specs and traceability: `docs/` (`requirements.md`, `design.md`, `tasks.md`, `validation.md`)

## Environment Notes
- Shell is PowerShell.
- Bash heredocs do not work; use PowerShell here-strings:
  ```powershell
  @'
  multi
  line
  '@
  ```

## Default Workflow
1. Read `docs/tasks.md` and keep checklist status accurate while implementing.
2. Keep changes small and parity-focused with AlbertsCave v1 behavior.
3. Prefer `rg` for search and `apply_patch` for focused edits.
4. Validate before handoff:
   - `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/validate-winui.ps1`
5. When finishing a numbered task, check it off in `docs/tasks.md` and create a commit.

## Guardrails
- Keep persistence local-only under `%LOCALAPPDATA%\BatCaveMonitor` (no outbound network behavior).
- Preserve CLI parity (`--print-gate-status`, `--benchmark`, and elevated helper mode).
- Keep UI logic in ViewModels/services; avoid moving behavior into code-behind unless strictly UI plumbing.
- Do not import or migrate persisted data from AlbertsCave.

## Continuous Improvement
- If a workaround is repeated, make a durable fix:
  - cross-project durable rule -> update `AGENTS.md`
  - task/tool-specific guidance -> add/update docs or script in `scripts/`
