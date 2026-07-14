import type { RuntimeAdminModeStatus, RuntimePersistence, RuntimeWarning } from "./types";

export type DiagnosticAction = "enable" | "retry";

export interface DiagnosticIssue {
  key: string;
  title: string;
  impact: string;
  action: DiagnosticAction | null;
  actionLabel: string | null;
  raw: string;
  occurredAtMs: number;
}

export function suppressedDiagnosticsLabel(persistence: RuntimePersistence | null): string {
  return persistence ? String(persistence.suppressed_diagnostic_events) : "Not reported";
}

export function currentDiagnosticIssues(
  warnings: RuntimeWarning[],
  adminMode: RuntimeAdminModeStatus,
  adminAvailable: boolean,
): DiagnosticIssue[] {
  const unique = new Map(warnings.map((warning) => [warning.key, warning]));
  return [...unique.values()]
    .reverse()
    .map((warning) => toDiagnosticIssue(warning, adminMode, adminAvailable));
}

export function uniqueWarningCount(warnings: RuntimeWarning[]): number {
  return new Set(warnings.map((warning) => warning.key)).size;
}

function toDiagnosticIssue(
  warning: RuntimeWarning,
  adminMode: RuntimeAdminModeStatus,
  adminAvailable: boolean,
): DiagnosticIssue {
  const value = `${warning.key} ${warning.category} ${warning.message}`.toLocaleLowerCase();
  const action = adminAction(adminMode, adminAvailable);

  if (value.includes("collector_service")) {
    return {
      ...baseIssue(warning),
      title: "Collector service needs attention",
      impact:
        "Standard monitoring remains current while service-only process fields may be unavailable.",
      action: null,
      actionLabel: null,
    };
  }

  if (warning.key === "admin_mode" || value.includes("admin_mode") || value.includes("elevat")) {
    return {
      ...baseIssue(warning),
      title:
        adminMode.state === "failed"
          ? "Privileged collection stopped"
          : "Privileged access needs attention",
      impact:
        "Standard monitoring remains current while restricted process fields may be unavailable.",
      ...action,
    };
  }

  if (value.includes("network_attribution") || value.includes("etw") || value.includes("ebpf")) {
    return {
      ...baseIssue(warning),
      title: "App network activity is unavailable",
      impact: "System network totals remain current, but app-level network values may be missing.",
      ...action,
    };
  }

  if (warning.category === "persistence" || value.includes("persistence_")) {
    return {
      ...baseIssue(warning),
      title: "Local data needs attention",
      impact:
        "Monitoring continues, but settings, warm cache, or diagnostics may remain session-only.",
      action: null,
      actionLabel: null,
    };
  }

  if (value.includes("permission") || value.includes("access") || value.includes("denied")) {
    return {
      ...baseIssue(warning),
      title: "Some process details are blocked",
      impact: "BatCave cannot read every field for protected processes.",
      ...action,
    };
  }

  return {
    ...baseIssue(warning),
    title: titleCase(warning.category || "Collector limitation"),
    impact: "A collector reported a limitation. Available telemetry continues to update.",
    action: null,
    actionLabel: null,
  };
}

function baseIssue(warning: RuntimeWarning) {
  return {
    key: warning.key,
    raw: warning.message,
    occurredAtMs: warning.occurred_at_ms,
  };
}

function adminAction(
  adminMode: RuntimeAdminModeStatus,
  adminAvailable: boolean,
): Pick<DiagnosticIssue, "action" | "actionLabel"> {
  if (
    !adminAvailable ||
    adminMode.collector_service != null ||
    adminMode.state === "unavailable" ||
    adminMode.state === "active" ||
    adminMode.state === "recovering"
  ) {
    return { action: null, actionLabel: null };
  }

  if (adminMode.state === "requesting") {
    return { action: null, actionLabel: null };
  }

  if (adminMode.state === "failed") {
    return { action: "retry", actionLabel: "Retry privileged access" };
  }

  return { action: "enable", actionLabel: "Enable privileged access" };
}

function titleCase(value: string): string {
  return value.replaceAll("_", " ").replace(/\b\w/g, (character) => character.toLocaleUpperCase());
}
