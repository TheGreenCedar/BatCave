# Native UI evidence

These screenshots show the native Apple Silicon app with live local telemetry on September 15, 2026. They cover Overview, Explore, and the compact workload inspector after the ranking, status-strip, and inspector changes in this revision.

| Capture | Details |
| --- | --- |
| Source | `9570141` (`devin/1789304882-ux-findings`) |
| App | BatCave Monitor `0.2.0-rc.5`, local `tauri dev` debug build |
| Host | Apple Silicon, macOS 27.0, build `26A428` |
| Appearance | Cave, system dark mode |
| Method | Direct native-window captures (`screencapture -l`) of the running Tauri window, downscaled from 2x to 1x |
| Window sizes | Overview and Explore: 1320 × 860; compact inspector: 781 × 768 |
| GUI SHA-256 | `f183778b475900301b94dd7008dfc92aa1f24c8c7626a8a4ef9c97968fb275d4` (debug binary) |

The images are unedited native captures, not browser fixtures or mockups.

## Overview

The status strip under the resource cards is always present, so a monitor warning changes its color rather than pushing Leading workloads down. Rankings settle: near-tied rows keep their positions between samples.

![Native Overview with machine resources, the status strip, and leading workloads](docs/images/batcave-monitor-macos-overview.jpg)

## Explore

The inspector shows the selected workload's history for all four resources as thin strips with their latest values. Selecting a strip expands it into the scrubbable chart. Technical details are limited to process identity.

![Native Explore with the Devin workload group selected and its CPU history expanded](docs/images/batcave-monitor-macos-explore.jpg)

## Compact inspector

At a narrow window width, workload details open in a drawer with the same history strips. Native verification covered expanding a strip, closing with Escape, and returning focus to the selected workload card.

![Native compact workload inspector with the CPU history expanded](docs/images/batcave-monitor-macos-compact-inspector.jpg)

The capture session ran with live process network activity. Diagnostics still lists unavailable macOS kernel CPU as a data limitation. These captures establish visible behavior; performance budgets and signed release verification remain separate requirements in [the implementation record](docs/rescue-implementation.md).
