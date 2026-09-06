<script lang="ts">
  import type { ChartPalette } from "./themes";
  import type { HistoryMetric, WorkloadHistoryPoint } from "./workloadInspection";
  export let retainedPoints = 0;
  export let historyTruncated = false;
  export let points: WorkloadHistoryPoint[] = [];
  // oxlint-disable-next-line no-unassigned-vars -- Svelte assigns this required component prop.
  export let activeTheme: ChartPalette;
  let metric: HistoryMetric = "cpu";
  let selectedSequence: number | null = null;
  const labels = { cpu: "CPU (one core)", memory: "Memory", io: "Read/write I/O", network: "Network" };
  const units = { cpu: "% of one core", memory: "bytes", io: "bytes/s", network: "bytes/s" };
  $: selected = points.find((point) => point.sample_seq === selectedSequence) ?? points.at(-1);
  $: selectedIndex = selected ? points.indexOf(selected) : 0;
  $: firstTime = points[0]?.sampled_at_ms ?? 0;
  $: lastTime = points.at(-1)?.sampled_at_ms ?? firstTime;
  $: span = Math.max(1000, lastTime - firstTime);
  $: ceiling = Math.max(metric === "cpu" ? 100 : 1, ...points.map((point) => point[metric].value ?? 0)) * 1.05;
  $: stroke = metric === "cpu" ? activeTheme.cpuStroke : metric === "memory" ? activeTheme.memoryStroke : metric === "io" ? activeTheme.diskReadStroke : activeTheme.networkDownStroke;
  $: path = chartPath(points, metric, firstTime, span, ceiling);
  function x(time: number): number { return 8 + ((time - firstTime) / span) * 584; }
  function chartPath(samples: WorkloadHistoryPoint[], resource: HistoryMetric, first: number, width: number, max: number): string {
    let drawing = false;
    return samples.map((point) => {
      const value = point[resource].value;
      if (value === null) { drawing = false; return ""; }
      const command = drawing && !point.gap_before ? "L" : "M";
      drawing = true;
      return `${command}${8 + ((point.sampled_at_ms - first) / width) * 584},${122 - (value / max) * 112}`;
    }).join(" ");
  }
  function pointAtPointer(event: PointerEvent): void {
    if (!(event.currentTarget instanceof SVGElement) || !points.length) return;
    const bounds = event.currentTarget.getBoundingClientRect();
    const at = firstTime + Math.max(0, Math.min(1, (event.clientX - bounds.left) / bounds.width)) * span;
    const nearest = points.reduce((best, point) => Math.abs(point.sampled_at_ms - at) < Math.abs(best.sampled_at_ms - at) ? point : best, points[0]);
    selectedSequence = nearest.sample_seq;
  }
  function chooseSample(event: Event): void { if (event.currentTarget instanceof HTMLInputElement) selectedSequence = points[Number(event.currentTarget.value)]?.sample_seq ?? null; }
  function time(value: number): string { return new Date(value).toLocaleTimeString(undefined, { hour12: false, hour: "2-digit", minute: "2-digit", second: "2-digit", fractionalSecondDigits: 3 }); }
</script>

<section class="inspection-history" aria-label="Workload history">
  <div class="history-heading"><label>Session history <select bind:value={metric} aria-label="History resource">{#each Object.entries(labels) as [value, label]}<option {value}>{label}</option>{/each}</select></label><small>{points.length} of {retainedPoints} samples</small></div>
  {#if historyTruncated}<p class="history-limit">Earlier samples are outside this retained window.</p>{/if}
  {#if selected}
    <svg viewBox="0 0 600 136" role="img" aria-label={`${labels[metric]} over recorded time; gaps represent missing measurements`} onpointermove={pointAtPointer}>
      <line x1="8" x2="592" y1="122" y2="122" stroke="currentColor" opacity="0.2" />
      <path d={path} fill="none" {stroke} stroke-width="2" vector-effect="non-scaling-stroke" />
      <line x1={x(selected.sampled_at_ms)} x2={x(selected.sampled_at_ms)} y1="6" y2="125" stroke="currentColor" opacity="0.45" stroke-dasharray="3 3" />
      {#if selected[metric].value !== null}<circle cx={x(selected.sampled_at_ms)} cy={122 - ((selected[metric].value ?? 0) / ceiling) * 112} r="3.5" fill={stroke} />{/if}
    </svg>
    <div class="history-times"><span>{time(firstTime)}</span><span>{time(lastTime)}</span></div>
    <input type="range" min="0" max={Math.max(0, points.length - 1)} value={selectedIndex} oninput={chooseSample} aria-label="Recorded sample" aria-valuetext={`${time(selected.sampled_at_ms)}: ${selected[metric].value === null ? "No measurement" : `${selected[metric].value} ${units[metric]}`}`} />
    <div class="history-readout"><time datetime={new Date(selected.sampled_at_ms).toISOString()}>{time(selected.sampled_at_ms)}</time><strong>{selected[metric].value === null ? "No measurement" : `${selected[metric].value} ${units[metric]}`}</strong><small>{selected[metric].quality} · {selected[metric].source.replaceAll("_", " ")} · {selected[metric].available} of {selected[metric].total} measured</small>{#if selected[metric].network_scope}<small>{selected[metric].network_scope?.replaceAll("_", " ")}</small>{/if}{#if selected.gap_before}<small>Start of a recorded segment</small>{/if}</div>
  {:else}<p>No samples retained for this identity.</p>{/if}
</section>
<style>
  .inspection-history { margin: 18px 0; min-width: 0; color: var(--text); font-size: 13px; }
  .history-heading, .history-times { display: flex; align-items: center; justify-content: space-between; gap: 10px; }
  .history-heading label { display: flex; align-items: center; gap: 8px; }
  select { color: inherit; background: var(--surface-1); border: 1px solid var(--border); border-radius: 5px; padding: 5px; max-width: 180px; }
  svg { width: 100%; height: auto; display: block; margin-top: 12px; }
  .history-times, small { font-size: 11px; color: var(--text-muted); }
  input { width: 100%; margin: 10px 0; }
  .history-readout { display: grid; gap: 5px; font-variant-numeric: tabular-nums; overflow-wrap: anywhere; }
  .history-limit { color: var(--text-muted); font-size: 12px; margin: 8px 0; }
  .history-readout time { font-size: 12px; }
  .history-readout strong { font-size: 14px; }
</style>
