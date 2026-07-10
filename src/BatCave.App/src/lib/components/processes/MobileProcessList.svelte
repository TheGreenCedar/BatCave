<script lang="ts">
  import { processSelectionKey, type ProcessIconKind } from "../../process";
  import { formatPercent, formatRate, processBytesLabel, processMemoryTitle } from "../../format";
  import type { ProcessSample, ProcessViewRow } from "../../types";
  import ProcessIcon from "./ProcessIcon.svelte";

  export let processRows: ProcessViewRow[] = [];
  export let selectedPid = "";
  export let processIcons: Record<string, string> = {};
  export let expandedGroups: Record<string, boolean> = {};
  export let onSelect: (pid: string) => void = () => {};
  export let onToggleGroup: (key: string) => void = () => {};

  $: cardRows = processRows.filter(
    (row) => row.kind === "group" || !row.is_grouped || (!!row.group_key && !!expandedGroups[row.group_key]),
  );

  function processCountLabel(count: number): string {
    return `${count} ${count === 1 ? "process" : "processes"}`;
  }

  function processForRow(row: ProcessViewRow): ProcessSample | undefined {
    return row.process ?? row.representative;
  }

  function iconSrc(process: ProcessSample | undefined): string | undefined {
    return process ? processIcons[process.exe || process.name] : undefined;
  }

  function iconKind(row: ProcessViewRow): ProcessIconKind {
    return (row.icon_kind as ProcessIconKind) || "process";
  }

  function selectedInRow(row: ProcessViewRow): boolean {
    return (
      (!!row.process && processSelectionKey(row.process) === selectedPid) ||
      (!!row.group_key && selectedPid === groupSelectionKey(row.group_key)) ||
      (!!row.group_key &&
        processRows.some(
          (candidate) =>
            candidate.group_key === row.group_key &&
            !!candidate.process &&
            processSelectionKey(candidate.process) === selectedPid,
        ))
    );
  }

  function selectRow(row: ProcessViewRow): void {
    if (row.kind === "group" && row.group_key) {
      onSelect(groupSelectionKey(row.group_key));
      return;
    }

    const process = processForRow(row);
    if (process) {
      onSelect(processSelectionKey(process));
    }
  }

  function groupSelectionKey(key: string): string {
    return `group:${key}`;
  }
</script>

<div class="mobile-process-list" aria-label="Attention queue cards">
  {#each cardRows as row}
    {@const process = processForRow(row)}
    {@const selected = selectedInRow(row)}
    {@const expanded = row.group_key ? !!expandedGroups[row.group_key] : false}
    <article
      class="mobile-process-card"
      class:selected={selected}
      class:child-card={row.kind === "process" && (row.is_grouped || row.is_child)}
    >
      <button
        class="mobile-card-select"
        type="button"
        aria-pressed={selected}
        aria-label={row.kind === "group" ? `Inspect ${row.group_label ?? "process"} group` : `Inspect ${process?.name}, PID ${process?.pid}`}
        onclick={() => selectRow(row)}
      >
        <span class="card-title-row">
          <span class="mobile-process-title">
            <ProcessIcon
              kind={iconKind(row)}
              child={row.kind === "process" && (row.is_grouped || row.is_child)}
              src={iconSrc(process)}
            />
            <span>
              <strong>{row.group_label ?? process?.name}</strong>
              <small>{row.kind === "group" ? `${processCountLabel(row.group_count)} / ${row.group_category}` : (row.group_category ?? `PID ${process?.pid}`)}</small>
            </span>
          </span>
          <small>{row.attention_label}</small>
        </span>
        <span class="card-metrics">
          <span>
            <em>CPU</em>
            <b>{formatPercent(row.cpu_percent)}</b>
          </span>
          <span>
            <em>Working set</em>
            <b title={process ? processMemoryTitle(process) : ""}>{process ? processBytesLabel(process, row.memory_bytes) : ""}</b>
          </span>
          <span>
            <em>I/O</em>
            <b>{formatRate(row.io_bps)}</b>
          </span>
        </span>
        <span class="card-foot">
          <span>{row.kind === "group" ? `${row.group_count} rows` : `PID ${process?.pid}`}</span>
          <span>{row.kind === "group" ? "grouped" : process?.status}</span>
        </span>
      </button>
      {#if row.kind === "group" && row.group_key}
        <button
          class="mobile-group-expand"
          type="button"
          aria-expanded={expanded}
          onclick={() => onToggleGroup(row.group_key ?? "")}
        >
          <svg class:expanded viewBox="0 0 16 16" aria-hidden="true"><path d="M5.5 3.5 10 8l-4.5 4.5" /></svg>
          {expanded ? "Collapse" : "Expand"} {processCountLabel(row.group_count)}
        </button>
      {/if}
    </article>
  {:else}
    <div class="mobile-empty-state">No process matches this view.</div>
  {/each}
</div>
