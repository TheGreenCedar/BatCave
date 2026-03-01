# Implementation Plan

- [x] 1. Establish solution structure and hosting foundation
  - [x] 1.1 Add `BatCave.Core` and `BatCave.Core.Tests` projects to `BatCave.slnx`.
  - [x] 1.2 Introduce Generic Host bootstrapping in `App.xaml.cs`.
  - [x] 1.3 Add packaged and unpackaged publish profiles and startup configuration wiring.
  - _Requirements: 10.1, 10.2, 10.3, 10.4_

- [x] 2. Implement launch policy gate and blocked-startup flow
  - [x] 2.1 Port Windows 11 startup gate logic and reason contract.
  - [x] 2.2 Wire startup gate evaluation before runtime loop activation.
  - [x] 2.3 Implement blocked-state projection in ViewModel and WinUI shell.
  - _Requirements: 1.1, 1.2, 1.3, 1.4_

- [ ] 3. Port Windows process collector
  - [ ] 3.1 Implement process snapshot enumeration and baseline row extraction.
  - [ ] 3.2 Implement CPU/IO/network-rate delta calculations.
  - [ ] 3.3 Implement access-state handling and partial metric fallback.
  - [ ] 3.4 Implement fallback start-time identity stabilization.
  - _Requirements: 2.1, 2.2, 2.3, 2.4_

- [ ] 4. Port telemetry pipeline and state store
  - [ ] 4.1 Implement identity-safe upsert/exit delta generation.
  - [ ] 4.2 Implement heartbeat emissions for unchanged rows.
  - [ ] 4.3 Implement warm-cache seed/reconcile flow.
  - [ ] 4.4 Implement in-memory row application and compaction hooks.
  - _Requirements: 3.1, 3.2, 3.3, 3.4_

- [ ] 5. Implement runtime loop, health accounting, and event gateway
  - [ ] 5.1 Implement 1-second scheduler with jitter and dropped-tick tracking.
  - [ ] 5.2 Implement generation-aware restart and loop cancellation flow.
  - [ ] 5.3 Emit telemetry, runtime health, and collector warning events.
  - [ ] 5.4 Integrate budget/degrade policy calculations into runtime state.
  - _Requirements: 4.1, 4.2, 4.3, 4.4_

- [ ] 6. Implement query, sorting, and filtering behavior
  - [ ] 6.1 Port query response projection (`seq`, `total`, `rows`) with paging.
  - [ ] 6.2 Port full sort-column and sort-direction contract.
  - [ ] 6.3 Port filter semantics for name/PID matching.
  - [ ] 6.4 Port incremental ordering updates and rebuild threshold behavior.
  - _Requirements: 5.1, 5.2, 5.3, 5.4_

- [ ] 7. Implement local persistence and diagnostics
  - [ ] 7.1 Port settings persistence to `%LOCALAPPDATA%\\BatCaveMonitor`.
  - [ ] 7.2 Port warm-cache load/save and startup hydration.
  - [ ] 7.3 Port structured local diagnostics logging and rotation behavior.
  - [ ] 7.4 Add guardrails/tests to verify local-only diagnostics posture.
  - _Requirements: 6.1, 6.2, 6.3, 6.4_

- [ ] 8. Implement admin-mode elevated bridge parity
  - [ ] 8.1 Add runtime admin toggle with backend restart semantics.
  - [ ] 8.2 Implement elevated helper launch/poll/token/stop-file lifecycle.
  - [ ] 8.3 Emit collector warnings and fault state propagation.
  - [ ] 8.4 Implement UI access filtering rules for admin/non-admin modes.
  - _Requirements: 7.1, 7.2, 7.3, 7.4_

- [ ] 9. Implement process metadata provider and ViewModel cache
  - [ ] 9.1 Implement metadata query contract (`parent_pid`, `command_line`, `executable_path`).
  - [ ] 9.2 Implement start-time validation and null-on-mismatch behavior.
  - [ ] 9.3 Implement non-fatal UI error presentation and loading states.
  - [ ] 9.4 Implement identity-keyed metadata cache lifecycle.
  - _Requirements: 8.1, 8.2, 8.3, 8.4_

- [ ] 10. Build native WinUI shell with feature parity
  - [ ] 10.1 Implement top bar interactions (filter, admin toggle, admin-only toggle).
  - [ ] 10.2 Implement virtualized process list with sortable columns.
  - [ ] 10.3 Implement detail panel metric focus and clear-selection flow.
  - [ ] 10.4 Implement runtime health footer and state-based shell rendering.
  - _Requirements: 9.1, 9.2, 9.3, 9.4_

- [ ] 11. Implement CLI operational modes and PowerShell scripts
  - [ ] 11.1 Implement `--print-gate-status` output and exit-code behavior.
  - [ ] 11.2 Implement `--benchmark` execution and summary projection.
  - [ ] 11.3 Implement strict benchmark gating thresholds.
  - [ ] 11.4 Add `scripts/run-dev.ps1` and `scripts/run-benchmark.ps1` for WinUI/.NET workflows.
  - _Requirements: 11.1, 11.2, 11.3, 11.4_

- [ ] 12. Validate parity, performance, and deployment completeness
  - [ ] 12.1 Add automated unit tests for pipeline, sort engine, bridge, and policy behavior.
  - [ ] 12.2 Add ViewModel-level tests for blocked/retry/admin/metadata scenarios.
  - [ ] 12.3 Verify packaged and unpackaged startup/runtime parity across architectures.
  - [ ] 12.4 Run benchmark and startup-gate CLI smoke checks from scripts.
  - _Requirements: 1.4, 3.4, 4.2, 7.3, 9.1, 10.4, 11.2, 11.4_
