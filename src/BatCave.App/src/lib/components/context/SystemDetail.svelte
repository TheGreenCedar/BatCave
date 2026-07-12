<script lang="ts">
  import MiniChart from "../../MiniChart.svelte";
  import { makeEmptySnapshot } from "../../runtimeSnapshot";
  import {
    driverCandidateLabel,
    formatBytes,
    formatPercent,
    formatRate,
    metricQualityLabel,
    optionalBytes,
    poolKindLabel,
    poolTagKey,
  } from "../../format";
  import type { DetailMode } from "../metrics/types";
  import type { ChartPalette } from "../../themes";
  import type { PlatformPresentation } from "../../platformPresentation";
  import type {
    KernelPoolTag,
    RuntimeSnapshot,
    SystemMemoryAccounting,
    SystemMetricQuality,
    TrendState,
  } from "../../types";

  export let detailMode: DetailMode;
  export let detailReadout: string;
  export let snapshot: RuntimeSnapshot = makeEmptySnapshot();
  export let history: TrendState;
  export let activeTheme: ChartPalette;
  export let presentation: PlatformPresentation;
  export let systemQuality: SystemMetricQuality = {};
  export let memoryPercent: number;
  export let swapPercent: number;
  export let memoryAccounting: SystemMemoryAccounting | undefined = undefined;
  export let topKernelPoolTags: KernelPoolTag[] = [];
  export let diskReadRate = 0;
  export let diskWriteRate = 0;
  export let networkDownRate = 0;
  export let networkUpRate = 0;
  export let diskScaleMax = 1_000_000;
  export let networkScaleMax = 750_000;
  export let coreLoads: { index: number; load: number; trend: number[] }[] = [];
  export let corePeak = 0;
  export let coreSpread = 0;
  export let hotCoreCount = 0;
  export let busyCoreCount = 0;
  export let coreTone: (load: number) => string;

  $: hottestCores = coreLoads.slice().sort((left, right) => right.load - left.load).slice(0, 8);
  $: hasSwap =
    snapshot.system.swap_total_bytes !== undefined &&
    snapshot.system.swap_total_bytes > 0 &&
    systemQuality.swap?.quality !== "unavailable";
  $: hasCommit = (memoryAccounting?.commit_limit_bytes ?? 0) > 0;
  $: commitPercent = percent(
    memoryAccounting?.commit_used_bytes ?? 0,
    memoryAccounting?.commit_limit_bytes ?? 0,
  );

  function percent(value: number, total: number): number {
    return total > 0 ? Math.min(100, Math.max(0, (value / total) * 100)) : 0;
  }
</script>

<section class="system-detail" aria-label="Resource detail view">
  {#if detailMode === "cpu"}
    <div class="quality-line">
      <span>CPU source</span><strong>{metricQualityLabel(systemQuality.cpu, "Measured")}</strong>
    </div>
    <div class="detail-summary compact-summary" aria-label="CPU summary">
      <div><span>Average</span><strong>{formatPercent(snapshot.system.cpu_percent)}</strong></div>
      <div><span>Peak core</span><strong>{formatPercent(corePeak)}</strong></div>
      <div><span>Hot cores</span><strong>{hotCoreCount}</strong></div>
      <div><span>Busy cores</span><strong>{busyCoreCount}</strong></div>
    </div>
    <div class="detail-hero-chart">
      <div><span>CPU pressure</span><strong>{detailReadout}</strong></div>
      <MiniChart values={history.cpu} max={100} stroke={activeTheme.cpuStroke} fill={activeTheme.cpuFill} />
    </div>
    <section class="core-distribution" aria-labelledby="core-distribution-title">
      <header><h3 id="core-distribution-title">Hottest logical cores</h3><span>{formatPercent(coreSpread)} spread</span></header>
      <div class="core-bars">
        {#each hottestCores as core (core.index)}
          <div class={`core-bar ${coreTone(core.load)}`}>
            <span>Core {core.index + 1}</span>
            <i><b style={`width: ${Math.min(100, Math.max(0, core.load))}%`}></b></i>
            <strong>{formatPercent(core.load)}</strong>
          </div>
        {/each}
      </div>
    </section>
  {:else if detailMode === "memory"}
    <div class="quality-line">
      <span>Memory source</span><strong>{metricQualityLabel(systemQuality.memory, "Measured")}</strong>
    </div>
    <div class="detail-summary compact-summary" aria-label="Memory summary">
      <div><span>Used</span><strong>{formatBytes(snapshot.system.memory_used_bytes)}</strong></div>
      <div><span>Available</span><strong>{optionalBytes(snapshot.system.memory_available_bytes)}</strong></div>
      <div><span>Load</span><strong>{formatPercent(memoryPercent)}</strong></div>
      <div>
        <span>{hasSwap ? "Swap" : hasCommit ? "Commit" : "Swap"}</span>
        <strong>{hasSwap ? formatPercent(swapPercent) : hasCommit ? formatPercent(commitPercent) : "Unavailable"}</strong>
      </div>
    </div>
    <div class="detail-chart-grid two-up compact-charts">
      <div class="detail-chart-card">
        <div><span>Memory load</span><strong>{formatPercent(memoryPercent)}</strong></div>
        <MiniChart values={history.memory} max={100} stroke={activeTheme.memoryStroke} fill={activeTheme.memoryFill} />
      </div>
      {#if hasSwap}
        <div class="detail-chart-card">
          <div><span>Swap load</span><strong>{formatPercent(swapPercent)}</strong></div>
          <MiniChart values={history.swap} max={100} stroke={activeTheme.swapStroke} fill={activeTheme.swapFill} />
        </div>
      {:else if hasCommit}
        <div class="detail-chart-card">
          <div><span>Commit load</span><strong>{formatPercent(commitPercent)}</strong></div>
          <p>
            {optionalBytes(memoryAccounting?.commit_used_bytes)} used of
            {optionalBytes(memoryAccounting?.commit_limit_bytes)}
          </p>
        </div>
      {:else}
        <div class="detail-chart-card unavailable-card">
          <div><span>Swap pressure</span><strong>Unavailable</strong></div>
          <p>{systemQuality.swap?.message ?? "This collector does not expose swap pressure."}</p>
        </div>
      {/if}
    </div>
    {#if memoryAccounting}
      <details class="technical-disclosure">
        <summary>Memory accounting</summary>
        <dl class="diagnostic-grid">
          <div><dt>{presentation.memoryLabel}</dt><dd>{formatBytes(memoryAccounting.process_working_set_bytes)}</dd></div>
          <div><dt>{presentation.privateMemoryLabel}</dt><dd>{formatBytes(memoryAccounting.process_private_bytes)}</dd></div>
          <div><dt>Blocked rows</dt><dd>{memoryAccounting.denied_process_count}</dd></div>
          <div><dt>Unattributed</dt><dd>{optionalBytes(memoryAccounting.unattributed_bytes)}</dd></div>
          <div><dt>Commit used</dt><dd>{optionalBytes(memoryAccounting.commit_used_bytes)}</dd></div>
          <div><dt>Commit limit</dt><dd>{optionalBytes(memoryAccounting.commit_limit_bytes)}</dd></div>
          <div><dt>Kernel paged</dt><dd>{optionalBytes(memoryAccounting.kernel_paged_pool_bytes)}</dd></div>
          <div><dt>Kernel nonpaged</dt><dd>{optionalBytes(memoryAccounting.kernel_nonpaged_pool_bytes)}</dd></div>
        </dl>
        {#if topKernelPoolTags.length > 0}
          <div class="compact-pool-list">
            {#each topKernelPoolTags as tag (poolTagKey(tag))}
              <div>
                <span><b>{tag.tag}</b> {poolKindLabel(tag.kind)}</span>
                <strong>{formatBytes(tag.bytes)}</strong>
                <small>{driverCandidateLabel(tag)}</small>
              </div>
            {/each}
          </div>
        {/if}
      </details>
    {/if}
  {:else if detailMode === "disk"}
    <div class="quality-line">
      <span>Disk source</span><strong>{metricQualityLabel(systemQuality.disk, "Aggregate")}</strong>
    </div>
    <div class="detail-summary compact-summary" aria-label="Disk summary">
      <div><span>Read</span><strong>{formatRate(diskReadRate)}</strong></div>
      <div><span>Write</span><strong>{formatRate(diskWriteRate)}</strong></div>
      <div><span>Read total</span><strong>{formatBytes(snapshot.system.disk_read_total_bytes)}</strong></div>
      <div><span>Write total</span><strong>{formatBytes(snapshot.system.disk_write_total_bytes)}</strong></div>
    </div>
    <div class="detail-chart-grid two-up compact-charts">
      <div class="detail-chart-card">
        <div><span>Read throughput</span><strong>{formatRate(diskReadRate)}</strong></div>
        <MiniChart values={history.diskRead} max={diskScaleMax} stroke={activeTheme.diskReadStroke} fill={activeTheme.diskReadFill} />
      </div>
      <div class="detail-chart-card">
        <div><span>Write throughput</span><strong>{formatRate(diskWriteRate)}</strong></div>
        <MiniChart values={history.diskWrite} max={diskScaleMax} stroke={activeTheme.diskWriteStroke} fill={activeTheme.diskWriteFill} />
      </div>
    </div>
  {:else}
    <div class="quality-line">
      <span>Network source</span><strong>{metricQualityLabel(systemQuality.network, "Aggregate")}</strong>
    </div>
    <div class="detail-summary compact-summary" aria-label="Network summary">
      <div><span>Download</span><strong>{formatRate(networkDownRate)}</strong></div>
      <div><span>Upload</span><strong>{formatRate(networkUpRate)}</strong></div>
      <div><span>Received</span><strong>{formatBytes(snapshot.system.network_received_total_bytes)}</strong></div>
      <div><span>Sent</span><strong>{formatBytes(snapshot.system.network_transmitted_total_bytes)}</strong></div>
    </div>
    <div class="detail-chart-grid two-up compact-charts">
      <div class="detail-chart-card">
        <div><span>Download rate</span><strong>{formatRate(networkDownRate)}</strong></div>
        <MiniChart values={history.netRx} max={networkScaleMax} stroke={activeTheme.networkDownStroke} fill={activeTheme.networkDownFill} />
      </div>
      <div class="detail-chart-card">
        <div><span>Upload rate</span><strong>{formatRate(networkUpRate)}</strong></div>
        <MiniChart values={history.netTx} max={networkScaleMax} stroke={activeTheme.networkUpStroke} fill={activeTheme.networkUpFill} />
      </div>
    </div>
  {/if}
</section>
