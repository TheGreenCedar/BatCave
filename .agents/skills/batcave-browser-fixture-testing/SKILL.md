---
name: batcave-browser-fixture-testing
description: How to run and UI-test the BatCave frontend in web-only fixture mode, and what the fixture can and cannot verify.
---

# BatCave browser fixture testing

## Start
- Requires Node >= 24. From `src/BatCave.App`: `npm install && npm run dev` (or `bash scripts/run-dev.sh --web-only` from the repo root). Serves `http://127.0.0.1:1420/`.
- If the page renders unstyled, the Vite CSS transform is stale: restart the dev server.
- Checks: `npm run typecheck`, `npm run lint`, `npm run format:check`, `npm run test:runtime-contract`, `npm run test:accessibility` (Playwright, uses the `?a11y=` fixtures below).

## Fixture entry points
- Default URL: dense deterministic fixture (~180 workloads).
- `?a11y=<state>` (dev only): small fixtures used by `scripts/accessibility.spec.ts` — `overview`, `process`, `group`, `settings`, `diagnostics`, `stale`, `degraded`, `compact`.

## What the fixture does NOT do
- Search/filter: the fixture echoes the query into settings but does not filter rows, so result counts under search cannot be validated.
- "Refresh now" while paused ingests a fresh fixture snapshot and resumes.
- Compact (<899px) mobile list: with many rows the list's `max-height: min(65vh, 680px)` grid squeezes cards to unreadable lines; item-tap flows are hard to exercise.
- Ranking is held while values change; click "Update order" to apply the pending order.
- All evidence is layout-only. Per AGENTS.md, native Tauri screenshots are required for PR evidence/product docs; label browser screenshots as layout-only.

## Recording tips
- Maximize Chrome with `wmctrl -r :ACTIVE: -b add,maximized_vert,maximized_horz` and record the actual CSS viewport size (`window.innerWidth/innerHeight`).
