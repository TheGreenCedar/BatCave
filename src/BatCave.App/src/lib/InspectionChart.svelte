<script lang="ts">
  import { formatBytes, formatPercent, formatRate } from "./format";
  import type { ChartPalette } from "./themes";
  import type { HistoryMetric, WorkloadHistoryPoint } from "./workloadInspection";
  export let points: WorkloadHistoryPoint[] = [];
  export let retainedPoints = 0;
  export let historyTruncated = false;
  // oxlint-disable-next-line no-unassigned-vars -- Svelte assigns this required component prop.
  export let activeTheme: ChartPalette;
  let expanded: HistoryMetric | null = null;
  let selectedSequence: number | null = null;
  const metrics: { key: HistoryMetric; label: string }[] = [
    { key: "cpu", label: "CPU" },
    { key: "memory", label: "Memory" },
    { key: "io", label: "Read/write I/O" },
    { key: "network", label: "Network" },
  ];
  $: selected = points.find((point) => point.sample_seq === selectedSequence) ?? points.at(-1);
  $: selectedIndex = selected ? points.indexOf(selected) : 0;
  $: firstTime = points[0]?.sampled_at_ms ?? 0;
  $: lastTime = points.at(-1)?.sampled_at_ms ?? firstTime;
  $: span = Math.max(1000, lastTime - firstTime);
  function ceilingFor(samples: WorkloadHistoryPoint[], metric: HistoryMetric): number {
    return (
      Math.max(
        metric === "cpu" ? 100 : 1,
        ...samples.map((point) => point[metric].value ?? 0),
      ) * 1.05
    );
  }

  // Collapsed rows rescale CPU to the observed peak so quiet series stay readable.
  function sparkCeilingFor(samples: WorkloadHistoryPoint[], metric: HistoryMetric): number {
    const peak = Math.max(0, ...samples.map((point) => point[metric].value ?? 0));
    return metric === "cpu" ? Math.max(10, peak * 1.2) : Math.max(1, peak) * 1.05;
  }

  function peakLabel(samples: WorkloadHistoryPoint[], metric: HistoryMetric): string {
    const values = samples.flatMap((point) => (point[metric].value === null ? [] : [point[metric].value ?? 0]));
    return values.length ? `peak ${formatValue(metric, Math.max(...values))}` : "";
  }

  function sampleCountLabel(shown: number, retained: number): string {
    return retained > shown
      ? `${shown} of ${retained} ${retained === 1 ? "sample" : "samples"}`
      : `${shown} ${shown === 1 ? "sample" : "samples"}`;
  }

  function windowSpanLabel(first: number, last: number): string {
    const spanMs = Math.max(0, last - first);
    if (spanMs < 90_000) return `last ${Math.max(1, Math.round(spanMs / 1000))} s`;
    return `last ${Math.round(spanMs / 60_000)} min`;
  }
  function strokeFor(theme: ChartPalette, metric: HistoryMetric): string {
    return metric === "cpu"
      ? theme.cpuStroke
      : metric === "memory"
        ? theme.memoryStroke
        : metric === "io"
          ? theme.diskReadStroke
          : theme.networkDownStroke;
  }
  function formatValue(metric: HistoryMetric, value: number): string {
    return metric === "cpu"
      ? formatPercent(value)
      : metric === "memory"
        ? formatBytes(value)
        : formatRate(value);
  }
  function formatLatest(samples: WorkloadHistoryPoint[], metric: HistoryMetric): string {
    const latest = samples.at(-1);
    if (!latest || latest[metric].value === null) return "—";
    return formatValue(metric, latest[metric].value ?? 0);
  }
  function x(at: number, first: number, width: number): number {
    return 8 + ((at - first) / width) * 584;
  }
  function chartPath(
    samples: WorkloadHistoryPoint[],
    resource: HistoryMetric,
    first: number,
    width: number,
    max: number,
    baseline: number,
    amplitude: number,
  ): string {
    let drawing = false;
    return samples
      .map((point) => {
        const value = point[resource].value;
        if (value === null) {
          drawing = false;
          return "";
        }
        const command = drawing && !point.gap_before ? "L" : "M";
        drawing = true;
        return `${command}${8 + ((point.sampled_at_ms - first) / width) * 584},${baseline - (value / max) * amplitude}`;
      })
      .join(" ");
  }
  function toggleMetric(metric: HistoryMetric): void {
    expanded = expanded === metric ? null : metric;
    selectedSequence = null;
  }
  function pointAtPointer(event: PointerEvent): void {
    if (!(event.currentTarget instanceof SVGElement) || !points.length) return;
    const bounds = event.currentTarget.getBoundingClientRect();
    const at =
      firstTime + Math.max(0, Math.min(1, (event.clientX - bounds.left) / bounds.width)) * span;
    const nearest = points.reduce(
      (best, point) =>
        Math.abs(point.sampled_at_ms - at) < Math.abs(best.sampled_at_ms - at) ? point : best,
      points[0],
    );
    selectedSequence = nearest.sample_seq;
  }
  function chooseSample(event: Event): void {
    if (event.currentTarget instanceof HTMLInputElement)
      selectedSequence = points[Number(event.currentTarget.value)]?.sample_seq ?? null;
  }
  function time(value: number): string {
    return new Date(value).toLocaleTimeString(undefined, {
      hour12: false,
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
      fractionalSecondDigits: 3,
    });
  }
</script>

<section class="inspection-history" aria-label="Workload history">
  <div class="history-heading">
    <h3>History</h3>
    <small
      >{sampleCountLabel(points.length, retainedPoints)}{points.length > 1
        ? ` · ${windowSpanLabel(firstTime, lastTime)}`
        : ""}</small
    >
  </div>
  <p class="history-hint">Select a row to expand.</p>
  {#if historyTruncated}<p class="history-limit">
      Earlier samples are outside this retained window.
    </p>{/if}
  {#each metrics as metric (metric.key)}
    {@const stroke = strokeFor(activeTheme, metric.key)}
    {@const ceiling = ceilingFor(points, metric.key)}
    {@const sparkCeiling = sparkCeilingFor(points, metric.key)}
    <button
      type="button"
      class="history-row"
      aria-expanded={expanded === metric.key}
      aria-label={`${metric.label} history`}
      onclick={() => toggleMetric(metric.key)}
    >
      <span class="history-row-label" title={metric.label}>{metric.label}</span>
      <svg class="history-sparkline" viewBox="0 0 600 40" aria-hidden="true">
        <path
          d={chartPath(points, metric.key, firstTime, span, sparkCeiling, 36, 32)}
          fill="none"
          {stroke}
          stroke-width="1.5"
          vector-effect="non-scaling-stroke"
        />
      </svg>
      <span class="history-row-value"
        >{formatLatest(points, metric.key)}<small class="history-row-peak"
          >{peakLabel(points, metric.key)}</small
        ></span
      >
    </button>
    {#if expanded === metric.key}
      <div class="history-detail">
        {#if selected}
          <svg
            viewBox="0 0 600 136"
            role="img"
            aria-label={`${metric.label} over recorded time; gaps represent missing measurements`}
            onpointermove={pointAtPointer}
          >
            <line x1="8" x2="592" y1="122" y2="122" stroke="currentColor" opacity="0.2" />
            <path
              d={chartPath(points, metric.key, firstTime, span, ceiling, 122, 112)}
              fill="none"
              {stroke}
              stroke-width="2"
              vector-effect="non-scaling-stroke"
            />
            <line
              x1={x(selected.sampled_at_ms, firstTime, span)}
              x2={x(selected.sampled_at_ms, firstTime, span)}
              y1="6"
              y2="125"
              stroke="currentColor"
              opacity="0.45"
              stroke-dasharray="3 3"
            />
            {#if selected[metric.key].value !== null}<circle
                cx={x(selected.sampled_at_ms, firstTime, span)}
                cy={122 - ((selected[metric.key].value ?? 0) / ceiling) * 112}
                r="3.5"
                fill={stroke}
              />{/if}
          </svg>
          <div class="history-times"><span>{time(firstTime)}</span><span>{time(lastTime)}</span></div>
          <input
            type="range"
            min="0"
            max={Math.max(0, points.length - 1)}
            value={selectedIndex}
            oninput={chooseSample}
            aria-label="Recorded sample"
            aria-valuetext={`${time(selected.sampled_at_ms)}: ${selected[metric.key].value === null ? "No measurement" : formatValue(metric.key, selected[metric.key].value ?? 0)}`}
          />
          <div class="history-readout">
            <time datetime={new Date(selected.sampled_at_ms).toISOString()}
              >{time(selected.sampled_at_ms)}</time
            >
            <strong
              >{selected[metric.key].value === null
                ? "No measurement"
                : formatValue(metric.key, selected[metric.key].value ?? 0)}</strong
            >
          </div>
        {:else}<p>No samples retained for this identity.</p>{/if}
      </div>
    {/if}
  {/each}
</section>
<style>
  .inspection-history {
    margin: 18px 0;
    min-width: 0;
    color: var(--text);
    font-size: 13px;
  }
  .history-heading,
  .history-times {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }
  .history-heading h3 {
    margin: 0;
    color: var(--text-soft);
    font-size: var(--text-sm);
    font-weight: 650;
  }
  .history-heading {
    margin-bottom: 6px;
  }
  .history-row {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    border: 0;
    border-top: 1px solid var(--border);
    padding: 8px 0;
    color: inherit;
    background: transparent;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .history-row-label {
    flex: 0 1 110px;
    min-width: 4.5rem;
    overflow: hidden;
    color: var(--text-soft);
    font-size: var(--text-xs);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .history-sparkline {
    flex: 1 1 0;
    min-width: 0;
    height: 40px;
    display: block;
  }
  .history-row-value {
    flex: 0 0 auto;
    display: flex;
    align-items: baseline;
    gap: 8px;
    font-variant-numeric: tabular-nums;
    font-family: var(--font-ui);
    font-size: var(--text-sm);
    text-align: right;
    white-space: nowrap;
  }
  .history-row-peak {
    color: var(--text-muted);
    font-size: var(--text-xs);
  }
  .history-hint {
    margin: 0 0 4px;
    color: var(--text-subtle);
    font-size: var(--text-xs);
  }
  .history-detail svg {
    width: 100%;
    height: auto;
    display: block;
    margin-top: 12px;
  }
  .history-times,
  small {
    font-size: 11px;
    color: var(--text-muted);
  }
  input {
    width: 100%;
    margin: 10px 0;
  }
  .history-readout {
    display: grid;
    gap: 5px;
    font-variant-numeric: tabular-nums;
    overflow-wrap: anywhere;
    padding-bottom: 8px;
  }
  .history-readout time {
    font-size: 12px;
  }
  .history-readout strong {
    font-size: 14px;
  }
  .history-limit {
    color: var(--text-muted);
    font-size: 12px;
    margin: 8px 0;
  }
</style>
