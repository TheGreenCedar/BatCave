# Telemetry DTO generation and IPC metadata

- Status: accepted for the version 3 transport design
- Date: 2026-07-13
- Scope: issue #66 spike only; no production contract migration

## Decision

Use `ts-rs` at a new, versioned Rust transport boundary, subject to the constraints below. Keep frontend view models separate and adapt the generated transport DTOs into them. Do not point `ts-rs` at the existing runtime structs or introduce a general schema framework.

Version 3 should carry metric meaning in one descriptor catalog per envelope. Each observation should carry only a descriptor index, value, quality code, and observation time. Repeated limitation text should live in a sparse catalog or side table and be referenced by index. The transport should retain an explicit protocol version and minimum-reader compatibility block.

The proposed value tuple is:

```text
[descriptor_index, value, quality_code, observed_at_ms, limitation_index?]
```

Generated declarations remain checked in as golden files. Rust tests must compare generated declarations with those files, and normal TypeScript checking must compile representative values against them.

## Evidence

The representative `ts-rs` fixture covers the selected indexed observation tuple, shared descriptor sources, a sparse limitation catalog, nested system/process/group DTOs, optional fields, `snake_case` enum values and fields, skipped fields, and an adjacently tagged process/group union. A checked JSON golden proves that the Rust serializer emits the same tuple, descriptor, quality-code, and limitation-index shape that the payload benchmark measures. The generated file also compiles in the normal Svelte TypeScript check.

The fixture also found two reasons not to generate the current runtime structs directly:

1. `#[serde(skip_serializing_if = "Option::is_none")]` without `#[serde(default)]` serializes as an omitted field but generates a required nullable TypeScript field. Existing structs use this pattern.
2. Rust `u64` generates as TypeScript `bigint`, while JSON transports carry an ordinary JSON number. Version 3 needs a transport timestamp type with a checked JavaScript-safe range or an explicit string representation.

Payload measurements are checked in at [`docs/evidence/dto-payload-spike-20260713.json`](../evidence/dto-payload-spike-20260713.json). The baseline starts from the checked version 2 `RuntimeSnapshot` fixture, expands every current `ProcessSample` field and quality block, and preserves the duplicated `ProcessViewRow` payload. Twenty percent of the processes form ten-member groups; the rest remain singleton view rows. The version 3 candidates carry every process once plus explicit group and system detail. At 5,000 process rows and 100 group rows:

| Strategy | Bytes | Change from v2 | Local parse p95 |
| --- | ---: | ---: | ---: |
| Current version 2 | 12,316,618 | baseline | 21.769 ms |
| Metadata on every value | 16,093,433 | +30.7% | 27.664 ms |
| Metadata repeated per family | 9,903,237 | -19.6% | 29.232 ms |
| Shared descriptor catalog | 4,018,609 | -67.4% | 11.013 ms |

Timings are directional local measurements. Serialized byte counts are deterministic for the fixture and are the primary architecture evidence.

Historical reproduction from `src/BatCave.App` at commit `8c61a008471a5a8a590672acf5bdf4d352fd8b2c`:

```bash
npm run test:dto-spike
npm run benchmark:dto-spike -- --write ../../docs/evidence/dto-payload-spike-20260713.json
cargo test --manifest-path src-tauri/Cargo.toml dto_spike
npm run typecheck
```

Those DTO-spike scripts belong to the recorded experiment and are no longer part of the current package surface. The checked evidence remains the output of that exact commit; current commands do not regenerate or supersede its four-strategy measurements.

Use the maintained production protocol guardrails from `src/BatCave.App` for current code:

```bash
npm run test:protocol-payload
npm run benchmark:protocol-payload
cargo test --manifest-path src-tauri/Cargo.toml generated_typescript_matches_checked_contract
cargo test --manifest-path src-tauri/Cargo.toml production_protocol_fixtures_match_encoder
npm run typecheck
```

These commands exercise the checked production version 3 envelope, its generated TypeScript contract, and its production fixtures. They are ongoing regression checks, not a reproduction of the historical spike.

## Metadata budget and guardrail

Use the 5,000-row fixture as the contract gate:

- serialized shared-catalog version 3 payload: at least 50% below the version 2 baseline at the same process/group workload;
- JSON encode and parse p95: no more than 3x the same-run version 2 baseline;
- descriptor meaning: semantic, scope, unit, interval, and source appear once per descriptor, never once per observation;
- quality and observation time remain per value because they can change independently;
- limitation text is sparse and referenced, never repeated for every value.

The checked shared-catalog result passes those size and local timing budgets. The focused test fails if it misses the 50% reduction. Timing should remain a reported benchmark until it can run on a stable host.

## Rejected options

- Continue handwritten Rust/TypeScript transport mirrors: low immediate cost, but it preserves the drift this spike is meant to remove.
- Generate the existing runtime structs in place: optional-field and `u64` behavior makes the declarations misleading without a contract cleanup.
- Repeat metadata per value: it is 30.7% larger than version 2 and fails the payload budget.
- Repeat metadata per family: removing duplicate view rows makes it smaller than version 2, but it is still 2.46 times the selected shared-catalog payload and repeats invariant meaning in every workload.
- Add JSON Schema, OpenAPI, Specta, or another general schema layer: no second schema consumer justifies the extra framework.
- Generate frontend view models: presentation needs differ from transport stability and would couple UI refactors to IPC versioning.

## Residual risks

- Compact tuples are less self-explanatory in raw payloads. The TypeScript adapter should expose named fields and reject unknown descriptor or quality indexes.
- A descriptor index is envelope-local. Cached values must not outlive their descriptor catalog.
- Protocol migration still needs dual-reader or coordinated cutover planning; this spike does not select the rollout sequence.
- `ts-rs` remains a build dependency whose generated output must be reviewed during upgrades.
