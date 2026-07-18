import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import {
  groupAttentionLabel,
  isProcessViewRow,
  processAttentionLabel,
  processNeedsAttention,
  processOtherIoRate,
} from "../src/lib/process.ts";
import {
  currentDiagnosticIssues,
  suppressedDiagnosticsLabel,
  uniqueWarningCount,
} from "../src/lib/diagnostics.ts";
import {
  collectorServiceStateLabel,
  installKindLabel,
  privilegedCollectionLabel,
  privilegedCollectionNote,
  privilegedSourceLabel,
  processElevationLabel,
} from "../src/lib/environmentPresentation.ts";
import { formatOptionalRate, qualityGuidance } from "../src/lib/format.ts";
import { hasNewRuntimeSample } from "../src/lib/runtimeSnapshot.ts";
import {
  dispatchAutomaticRuntimeHydration,
  planAutomaticRuntimeFocusHydration,
} from "../src/lib/runtimeHydration.ts";
import { RuntimeMutationQueue } from "../src/lib/tauriBridge.ts";
import { UiPreferencePersistenceSequence } from "../src/lib/uiPreferencePersistence.ts";
import type {
  ProcessSample,
  GroupDetail,
  ProcessViewRow,
  RuntimeAdminModeStatus,
  RuntimeCollectorServiceState,
  RuntimeCollectorServiceStatus,
  RuntimeEnvironment,
  RuntimeSnapshot,
  RuntimeWarning,
} from "../src/lib/types.ts";

const canonicalSnapshot = JSON.parse(
  readFileSync(new URL("./fixtures/runtime-snapshot.v2.json", import.meta.url), "utf8"),
);
const provenanceFixtures = JSON.parse(
  readFileSync(new URL("./fixtures/runtime-provenance.json", import.meta.url), "utf8"),
) as Array<{
  name: string;
  environment: RuntimeEnvironment;
  admin_mode: RuntimeAdminModeStatus;
  expected_process_label: string;
  expected_collection_label: string;
  expected_package_label: string;
}>;
const workloadDetails = JSON.parse(
  readFileSync(new URL("./fixtures/workload-details.v1.json", import.meta.url), "utf8"),
);
const themeCss = readFileSync(new URL("../src/styles/themes.css", import.meta.url), "utf8");
const releaseManifest = readFileSync(
  new URL("../src-tauri/release.manifest.xml", import.meta.url),
  "utf8",
);
const tauriBuildScript = readFileSync(new URL("../src-tauri/build.rs", import.meta.url), "utf8");

function process(overrides: Partial<ProcessSample> = {}): ProcessSample {
  return {
    pid: "1",
    parent_pid: null,
    start_time_ms: 1,
    name: "quiet.exe",
    exe: "C:\\quiet.exe",
    status: "idle",
    cpu_percent: 0,
    memory_bytes: 1,
    private_bytes: 1,
    io_read_total_bytes: 0,
    io_write_total_bytes: 0,
    io_read_bps: 0,
    io_write_bps: 0,
    other_io_bps: 0,
    network_received_bps: 0,
    network_transmitted_bps: 0,
    threads: 1,
    handles: 1,
    access_state: "full",
    quality: {
      cpu: { quality: "native", source: "direct_api" },
      memory: { quality: "native", source: "direct_api" },
      io: { quality: "native", source: "direct_api" },
      network: { quality: "native", source: "direct_api" },
    },
    ...overrides,
  };
}

test("control publications do not look like new telemetry samples", () => {
  assert.equal(hasNewRuntimeSample({ sample_seq: 7 }, { sample_seq: 7 }), false);
  assert.equal(hasNewRuntimeSample({ sample_seq: 7 }, { sample_seq: 8 }), true);
});

test("suppressed diagnostics stay unknown when persistence is not reported", () => {
  assert.equal(suppressedDiagnosticsLabel(null), "Not reported");
  assert.equal(
    suppressedDiagnosticsLabel({
      state: "healthy",
      roots: [],
      components: [],
      suppressed_diagnostic_events: 0,
    }),
    "0",
  );
});

function durablePreferenceSnapshot(theme: string, historyPointLimit: number): RuntimeSnapshot {
  return {
    ...canonicalSnapshot,
    settings: {
      ...canonicalSnapshot.settings,
      ui_preferences: {
        theme,
        history_point_limit: historyPointLimit,
      },
    },
    persistence: {
      state: "healthy",
      roots: [],
      components: [
        {
          owner: "current_user",
          kind: "settings",
          state: "healthy",
          durability: "durable",
          last_success_at_ms: 10,
          active_failure: null,
        },
      ],
      suppressed_diagnostic_events: 0,
    },
  } as RuntimeSnapshot;
}

test("rapid UI preference saves keep the newest fallback through out-of-order responses", () => {
  const sequence = new UiPreferencePersistenceSequence();
  const first = sequence.begin({ theme: "cave", history_point_limit: 72 });
  const latest = sequence.begin({ theme: "ember", history_point_limit: 180 });
  let fallback: typeof latest.preferences | null = { ...latest.preferences };

  if (sequence.isLatestDurable(first, durablePreferenceSnapshot("cave", 72))) {
    fallback = null;
  }
  assert.equal(sequence.isLatest(first), false, "older failures are stale too");
  assert.deepEqual(
    fallback,
    latest.preferences,
    "an older durable response cannot clear newer state",
  );

  // The latest request fails, so it has no durable acknowledgement and the fallback survives.
  assert.deepEqual(fallback, latest.preferences);
  assert.equal(
    sequence.isLatestDurable(latest, durablePreferenceSnapshot("ember", 72)),
    false,
    "a mismatched acknowledgement cannot clear the fallback",
  );
  assert.equal(
    sequence.isLatestDurable(latest, durablePreferenceSnapshot("ember", 180)),
    true,
    "only the newest matching durable pair can clear the fallback",
  );
});

test("runtime mutation queue preserves invocation order and continues after failure", async () => {
  const queue = new RuntimeMutationQueue();
  const invocations: string[] = [];
  let releaseFirst = () => {};
  const firstGate = new Promise<void>((resolve) => {
    releaseFirst = resolve;
  });

  const first = queue.run(async () => {
    invocations.push("first");
    await firstGate;
    return 1;
  });
  const second = queue.run(async () => {
    invocations.push("second");
    return 2;
  });

  await Promise.resolve();
  assert.deepEqual(invocations, ["first"]);
  releaseFirst();
  assert.deepEqual(await Promise.all([first, second]), [1, 2]);
  assert.deepEqual(invocations, ["first", "second"]);

  await assert.rejects(
    queue.run(async () => Promise.reject(new Error("expected"))),
    /expected/u,
  );
  assert.equal(await queue.run(async () => 3), 3);

  const bounded = new RuntimeMutationQueue(2);
  let releaseBounded = () => {};
  const boundedGate = new Promise<void>((resolve) => {
    releaseBounded = resolve;
  });
  const held = bounded.run(async () => boundedGate);
  const queued = bounded.run(async () => 2);
  await assert.rejects(
    bounded.run(async () => 3),
    (error) => error === "runtime_control_busy",
  );
  releaseBounded();
  await held;
  assert.equal(await queued, 2);
});

test("automatic focus hydration keeps the published control until its query applies", () => {
  const published = durablePreferenceSnapshot("system", 72);
  published.settings.query.focus_mode = "all";

  assert.deepEqual(planAutomaticRuntimeFocusHydration(published, true), {
    desired: "attention",
    requiresSync: true,
    visible: "all",
  });
  assert.deepEqual(planAutomaticRuntimeFocusHydration(published, false), {
    desired: "all",
    requiresSync: false,
    visible: "all",
  });
});

test("degraded first-snapshot hydration dispatches only the runtime-only query", () => {
  const degraded = durablePreferenceSnapshot("system", 72);
  degraded.settings.ui_preferences = null;
  degraded.persistence = {
    state: "degraded",
    roots: [],
    components: [
      {
        owner: "current_user",
        kind: "settings",
        state: "degraded",
        durability: "session_only",
        last_success_at_ms: null,
        active_failure: {
          code: "corrupt_data",
          operation: "parse",
          occurred_at_ms: 10,
          retryable: false,
          summary: "settings JSON is corrupt",
        },
      },
    ],
    suppressed_diagnostic_events: 0,
  };
  const nativeInvocations: string[] = [];

  dispatchAutomaticRuntimeHydration(
    degraded,
    { persistUiPreferences: true, syncRuntimeQuery: true },
    {
      persistUiPreferences: () => nativeInvocations.push("set_ui_preferences"),
      syncRuntimeQuery: () => nativeInvocations.push("set_process_query:runtime_only"),
    },
  );

  assert.deepEqual(nativeInvocations, ["set_process_query:runtime_only"]);

  dispatchAutomaticRuntimeHydration(
    degraded,
    { persistUiPreferences: true, syncRuntimeQuery: false },
    {
      persistUiPreferences: () => nativeInvocations.push("set_ui_preferences"),
      syncRuntimeQuery: () => nativeInvocations.push("set_process_query:runtime_only"),
    },
  );
  assert.deepEqual(nativeInvocations, ["set_process_query:runtime_only"]);

  dispatchAutomaticRuntimeHydration(
    durablePreferenceSnapshot("system", 72),
    { persistUiPreferences: true, syncRuntimeQuery: true },
    {
      persistUiPreferences: () => nativeInvocations.push("set_ui_preferences"),
      syncRuntimeQuery: () => nativeInvocations.push("set_process_query:runtime_only"),
    },
  );
  assert.deepEqual(nativeInvocations, [
    "set_process_query:runtime_only",
    "set_ui_preferences",
    "set_process_query:runtime_only",
  ]);
});

test("shared fixture exposes the preview environment and stable empty arrays", () => {
  assert.deepEqual(canonicalSnapshot.environment, {
    platform: "windows",
    admin_mode_available: true,
    process_elevation: "standard",
    install_kind: "portable",
    data_directory: "C:\\Users\\test\\BatCaveMonitor",
  });
  assert.deepEqual(
    canonicalSnapshot.system.memory_accounting.kernel_pool_tags[0].driver_candidates,
    [],
  );
  assert.equal(canonicalSnapshot.seq, undefined);
  assert.equal(canonicalSnapshot.ts_ms, undefined);
  assert.equal(canonicalSnapshot.standard_fallback_process_etw_disabled, false);
  assert.deepEqual(canonicalSnapshot.admin_mode, {
    state: "off",
    source: "none",
    detail: null,
    last_success_at_ms: null,
  });
});

test("provenance fixtures keep package and privilege copy deterministic", () => {
  assert.deepEqual(
    provenanceFixtures.map((fixture) => [
      fixture.name,
      processElevationLabel(fixture.environment),
      privilegedCollectionLabel(fixture.admin_mode),
      installKindLabel(fixture.environment.install_kind),
    ]),
    provenanceFixtures.map((fixture) => [
      fixture.name,
      fixture.expected_process_label,
      fixture.expected_collection_label,
      fixture.expected_package_label,
    ]),
  );

  const unavailable = provenanceFixtures.find(
    (fixture) => fixture.name === "windows_provenance_and_token_unavailable",
  );
  assert.ok(unavailable);
  assert.equal(
    privilegedCollectionNote(unavailable.admin_mode),
    "Privileged collection is inactive; standard monitoring remains available.",
  );
});

test("shipped Windows release starts as the invoking user", () => {
  assert.match(releaseManifest, /requestedExecutionLevel level="asInvoker"/);
  assert.doesNotMatch(releaseManifest, /requireAdministrator/);
});

test("Windows binaries and test executables embed the Common-Controls manifest", () => {
  assert.match(releaseManifest, /name="Microsoft\.Windows\.Common-Controls"/);
  assert.match(releaseManifest, /version="6\.0\.0\.0"/);
  assert.match(tauriBuildScript, /WindowsAttributes::new_without_app_manifest\(\)/);
  assert.match(tauriBuildScript, /\.join\("release\.manifest\.xml"\)/);
  assert.match(tauriBuildScript, /cargo:rustc-link-arg=\/MANIFEST:EMBED/);
  assert.match(tauriBuildScript, /cargo:rustc-link-arg=\/MANIFESTINPUT:/);
});

test("collector-service states stay explicit", () => {
  const expectedLabels: Record<RuntimeCollectorServiceState, string> = {
    not_installed: "Collector service not installed",
    stopped: "Collector service stopped",
    connecting: "Collector service connecting",
    recovering: "Collector service recovering",
    active: "Collector service active",
    incompatible: "Collector service incompatible",
    unauthorized: "Collector service unauthorized",
    failed: "Collector service failed",
  };

  for (const [state, expectedLabel] of Object.entries(expectedLabels) as Array<
    [RuntimeCollectorServiceState, string]
  >) {
    const status = collectorService(state);
    const mode: RuntimeAdminModeStatus = {
      state: state === "active" ? "active" : state === "recovering" ? "recovering" : "off",
      source: state === "active" ? "collector_service" : "none",
      detail: null,
      last_success_at_ms: state === "active" ? 1_783_944_001_000 : null,
      collector_service: status,
    };

    assert.equal(collectorServiceStateLabel(status), expectedLabel);
    assert.equal(privilegedCollectionLabel(mode), expectedLabel);
    assert.match(
      privilegedCollectionNote(mode),
      state === "active" ? /standard token/ : /standard monitoring remains current/,
    );
  }

  const active: RuntimeAdminModeStatus = {
    state: "active",
    source: "collector_service",
    detail: null,
    last_success_at_ms: 1_783_944_001_000,
    collector_service: collectorService("active"),
  };
  assert.equal(privilegedCollectionLabel(active, 2), "Collector service active, 2 blocked");
  assert.equal(privilegedSourceLabel(active.source), "Installed collector service");
});

test("collector-service warnings remain actionless in diagnostics", () => {
  const mode: RuntimeAdminModeStatus = {
    state: "failed",
    source: "none",
    detail: "collector_service_unauthorized",
    last_success_at_ms: null,
    collector_service: collectorService("unauthorized"),
  };
  const issue = currentDiagnosticIssues(
    [warning("collector_service.unauthorized", "collector_service_unauthorized", 1)],
    mode,
    true,
  )[0];

  assert.equal(issue.title, "Collector service needs attention");
  assert.equal(issue.action, null);
  assert.match(issue.impact, /Standard monitoring remains current/);
});

test("shared workload fixture keeps process and group details disjoint", () => {
  assert.equal(workloadDetails.length, 2);
  assert.ok(workloadDetails.every(isProcessViewRow));

  const processAsGroup = structuredClone(workloadDetails[0]);
  processAsGroup.kind = "group";
  assert.equal(isProcessViewRow(processAsGroup), false);

  const groupAsProcess = structuredClone(workloadDetails[1]);
  groupAsProcess.kind = "process";
  assert.equal(isProcessViewRow(groupAsProcess), false);

  const representativeAggregate = structuredClone(workloadDetails[1]);
  representativeAggregate.detail.pid = "42";
  representativeAggregate.detail.exe = "/usr/bin/code";
  representativeAggregate.detail.access_state = "full";
  assert.equal(isProcessViewRow(representativeAggregate), false);
});

test("diagnostics render one limitation per stable key with the current admin action", () => {
  const warnings: RuntimeWarning[] = [
    warning("collector.network_attribution", "network_attribution_failed: access denied", 1),
    warning("collector.network_attribution", "network_attribution_failed: retry failed", 2),
  ];

  assert.deepEqual(
    currentDiagnosticIssues(warnings, adminMode("off"), true).map((issue) => [
      issue.key,
      issue.action,
    ]),
    [["collector.network_attribution", "enable"]],
  );
  const requesting = currentDiagnosticIssues(warnings, adminMode("requesting"), true)[0];
  assert.equal(requesting.action, null);
  assert.equal(requesting.actionLabel, null);
  assert.equal(currentDiagnosticIssues(warnings, adminMode("failed"), true)[0].action, "retry");
  assert.equal(currentDiagnosticIssues(warnings, adminMode("active"), true)[0].action, null);
  assert.equal(uniqueWarningCount(warnings), 1);
});

test("persistence failures explain session-only state without a process-access action", () => {
  const issue = currentDiagnosticIssues(
    [
      {
        ...warning("persistence.storage_full", "persistence_storage_full operation=write", 1),
        category: "persistence",
      },
    ],
    adminMode("off"),
    true,
  )[0];

  assert.equal(issue.title, "Local data needs attention");
  assert.match(issue.impact, /session-only/);
  assert.equal(issue.action, null);
});

test("native metrics omit empty quality guidance", () => {
  assert.deepEqual(
    qualityGuidance({
      cpu: { quality: "native", source: "direct_api" },
      disk: { quality: "native", source: "pdh" },
      network: { quality: "native", source: "interface_aggregate" },
    }),
    [],
  );
  assert.deepEqual(
    qualityGuidance({ network: { quality: "unavailable", message: "ETW access denied" } }),
    ["ETW access denied"],
  );
});

test("attention includes each scored resource and limited access", () => {
  const quiet = process();

  assert.equal(processNeedsAttention(quiet), false);
  assert.equal(processNeedsAttention(process({ cpu_percent: 10 })), true);
  assert.equal(processNeedsAttention(process({ memory_bytes: 900 * 1024 * 1024 })), true);
  assert.equal(processNeedsAttention(process({ io_read_bps: 500 * 1024 })), true);
  assert.equal(processNeedsAttention(process({ network_received_bps: 1024 * 1024 })), true);
  assert.equal(processNeedsAttention(process({ access_state: "partial" })), true);
  assert.equal(processNeedsAttention(process({ other_io_bps: 8 * 1024 * 1024 })), false);
});

test("singleton attention labels publish only quality-backed activity", () => {
  const quality = (value: "native" | "estimated" | "partial" | "held" | "unavailable") => {
    const metric = { quality: value, source: "direct_api" as const };
    return { cpu: metric, memory: metric, io: metric, network: metric };
  };

  assert.equal(processAttentionLabel(process({ cpu_percent: 9 })), "steady");
  assert.equal(processAttentionLabel(process({ cpu_percent: 10 })), "CPU activity");
  assert.equal(
    processAttentionLabel(process({ cpu_percent: 90, quality: quality("held") })),
    "Pending",
  );
  assert.equal(
    processAttentionLabel(process({ cpu_percent: 90, quality: quality("unavailable") })),
    "Unavailable",
  );
  assert.equal(processAttentionLabel(process({ cpu_percent: 90, quality: undefined })), "Limited");
  assert.equal(
    processAttentionLabel(process({ cpu_percent: 90, quality: quality("partial") })),
    "CPU activity · limited",
  );
  assert.equal(
    processAttentionLabel(process({ cpu_percent: 90, quality: quality("estimated") })),
    "CPU activity · estimated",
  );
  assert.equal(
    processAttentionLabel(
      process({
        cpu_percent: 0,
        network_received_bps: 2 * 1024 * 1024,
        quality: quality("partial"),
      }),
    ),
    "network activity · limited",
  );

  const unsupportedNetwork = quality("native");
  unsupportedNetwork.cpu = { quality: "estimated", source: "direct_api" };
  unsupportedNetwork.network = { quality: "unavailable", source: "direct_api" };
  assert.equal(
    processAttentionLabel(process({ cpu_percent: 4, quality: unsupportedNetwork })),
    "Limited",
  );
});

test("fixture group attention exposes nonpublishable and mixed coverage states", () => {
  const detail = (
    quality: "native" | "partial" | "held" | "unavailable",
    available: number,
    cpuPercent = 0,
  ): GroupDetail => {
    const metric = { quality, source: "process_aggregate" as const };
    const coverage = { available, total: 2 };
    return {
      kind: "group",
      workload_id: "group:searchindexer.exe",
      group_key: "searchindexer.exe",
      label: "SearchIndexer.exe",
      category: "Windows",
      process_count: 2,
      cpu_percent: cpuPercent,
      memory_bytes: 0,
      io_bps: 0,
      network_bps: 0,
      threads: 0,
      quality: {
        cpu: metric,
        memory: metric,
        io: metric,
        other_io: { quality: "unavailable", source: "process_aggregate" },
        network: metric,
        threads: metric,
      },
      coverage: {
        cpu: coverage,
        memory: coverage,
        io: coverage,
        other_io: { available: 0, total: 2 },
        network: coverage,
        threads: coverage,
      },
    };
  };

  assert.equal(groupAttentionLabel(detail("held", 0), false), "Pending · 0/2 coverage");
  assert.equal(groupAttentionLabel(detail("unavailable", 0), false), "Unavailable · 0/2 coverage");
  assert.equal(groupAttentionLabel(detail("partial", 0), false), "Limited · 0/2 coverage");

  const mixed = detail("partial", 1, 10);
  assert.deepEqual(mixed.coverage.cpu, { available: 1, total: 2 });
  assert.equal(groupAttentionLabel(mixed, false), "CPU activity · 1/2 · limited");
});

test("typed groups keep Other I/O separate and unavailable", () => {
  const group = (workloadDetails as ProcessViewRow[]).find((row) => row.kind === "group");
  assert.ok(group?.kind === "group");
  assert.equal(group.detail.other_io_bps, undefined);
  assert.equal(group.detail.quality.other_io.quality, "unavailable");
  assert.deepEqual(group.detail.coverage.other_io, { available: 0, total: 2 });
  assert.equal(formatOptionalRate(group.detail.other_io_bps), "Unavailable");
  assert.equal(formatOptionalRate(processOtherIoRate(process({ other_io_bps: 0 }), {})), "0 B/s");
});

test("all theme text and focus colors meet contrast floors", () => {
  const blocks = [
    themeCss.match(/\.app-shell,\s*:root\s*\{([^}]*)\}/s)?.[1],
    ...[...themeCss.matchAll(/\.app-shell\[data-theme="[^"]+"\]\s*\{([^}]*)\}/gs)].map(
      (match) => match[1],
    ),
  ];

  assert.equal(blocks.length, 4);
  for (const block of blocks) {
    assert.ok(block);
    const variables = Object.fromEntries(
      [...block.matchAll(/--([\w-]+):\s*(#[\da-f]{6})/gi)].map((match) => [match[1], match[2]]),
    );
    for (const surface of ["surface-0", "surface-1", "surface-2", "surface-3"]) {
      assert.ok(contrast(variables["text-subtle"], variables[surface]) >= 4.5);
      assert.ok(contrast(variables.accent, variables[surface]) >= 3);
    }
  }
});

function contrast(left: string, right: string): number {
  const [lighter, darker] = [luminance(left), luminance(right)].sort((a, b) => b - a);
  return (lighter + 0.05) / (darker + 0.05);
}

function luminance(color: string): number {
  const channels = [1, 3, 5].map(
    (index) => Number.parseInt(color.slice(index, index + 2), 16) / 255,
  );
  const [red, green, blue] = channels.map((value) =>
    value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4,
  );
  return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
}

function warning(key: string, message: string, publicationSeq: number): RuntimeWarning {
  return {
    key,
    message,
    publication_seq: publicationSeq,
    occurred_at_ms: publicationSeq,
    category: "collector",
  };
}

function adminMode(state: RuntimeAdminModeStatus["state"]): RuntimeAdminModeStatus {
  return {
    state,
    source: "none",
    detail: null,
    last_success_at_ms: null,
    collector_service: null,
  };
}

function collectorService(state: RuntimeCollectorServiceState): RuntimeCollectorServiceStatus {
  return {
    state,
    release_identity:
      state === "active" ? { app_version: "1.0.0", source_commit_sha: "abc" } : null,
    service_version: state === "active" || state === "incompatible" ? "1.0.0" : null,
    negotiated_protocol_version: state === "active" ? 3 : null,
    minimum_desktop_version: null,
    instance_id: state === "active" ? "collector-instance" : null,
    last_connected_at_ms: state === "active" ? 1_783_944_001_000 : null,
    detail: state === "active" ? null : `collector_service_${state}`,
  };
}
