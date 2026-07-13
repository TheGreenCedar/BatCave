import type {
  ExistingOptionalWithoutDefault,
  RuntimeEnvelope,
  SkippedInternalField,
} from "./generated/dto-spike";

export const representativeRuntimeEnvelope = {
  protocol_version: 2,
  event_kind: "runtime_snapshot",
  compatibility: {
    minimum_reader_version: 2,
    breaking: false,
  },
  descriptors: [
    {
      id: "process.cpu.usage",
      semantic: "cpu_usage",
      scope: "process",
      unit: "percent_one_core",
      interval_ms: 1_000,
    },
  ],
  workloads: [
    {
      kind: "process",
      detail: {
        stable_id: "process:42:1700000000000",
        pid: "42",
        display_name: "fixture.exe",
        metrics: [
          {
            descriptor_id: "process.cpu.usage",
            value: 12.5,
            quality: "native",
            observed_at_ms: 1_720_000_000_000n,
          },
        ],
      },
    },
    {
      kind: "group",
      detail: {
        stable_id: "group:fixture",
        display_name: "Fixture group",
        coverage: {
          included_processes: 1,
          total_processes: 1,
        },
        metrics: [],
      },
    },
  ],
} satisfies RuntimeEnvelope;

// ts-rs makes this field required because the matching Rust field uses
// skip_serializing_if without serde(default). The Rust test proves serde omits it.
export const currentOptionalPattern = {
  required_name: "current-pattern",
  optional_message: null,
} satisfies ExistingOptionalWithoutDefault;

export const skippedFieldPattern = {
  visible_name: "public",
} satisfies SkippedInternalField;
