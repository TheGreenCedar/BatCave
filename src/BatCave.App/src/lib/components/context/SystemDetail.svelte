<script lang="ts">
  import MiniChart from "../../MiniChart.svelte";
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
  import type { KernelPoolTag, RuntimeSnapshot, SystemMemoryAccounting, SystemMetricQuality, TrendState } from "../../types";

  export let detailMode: DetailMode;
  export let detailTitle: string;
  export let detailReadout: string;
  export let snapshot: RuntimeSnapshot;
  export let history: TrendState;
  export let activeTheme: ChartPalette;
  export let systemQuality: SystemMetricQuality;
  export let memoryPercent: number;
  export let swapPercent: number;
  export let memoryAccounting: SystemMemoryAccounting | undefined;
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
</script>

<section id="resource-detail-panel" class="system-detail" aria-label="Resource detail view">
  <div class="panel-heading">
    <div>
      <span class="section-label">System detail</span>
      <h2 tabindex="-1">{detailTitle}</h2>
    </div>
    <strong>{detailReadout}</strong>
  </div>
  {#if detailMode === "cpu"}
    <div class="detail-summary" aria-label="CPU distribution summary">
      <div><span>Peak</span><strong>{formatPercent(corePeak)}</strong></div>
      <div><span>Hot cores</span><strong>{hotCoreCount}</strong></div>
      <div><span>Busy</span><strong>{busyCoreCount}</strong></div>
      <div><span>Spread</span><strong>{formatPercent(coreSpread)}</strong></div>
    </div>
    <div class="core-timeseries" aria-label="Logical core time series">
      {#each coreLoads as core}
        <div class={`core-trend-card ${coreTone(core.load)}`}>
          <div>
            <span>Core {core.index + 1}</span>
            <strong>{formatPercent(core.load)}</strong>
          </div>
          <MiniChart values={core.trend} max={100} stroke={activeTheme.cpuStroke} fill={activeTheme.cpuFill} />
        </div>
      {/each}
    </div>
  {:else if detailMode === "memory"}
    <div class="detail-summary" aria-label="Memory summary">
      <div><span>Used</span><strong>{formatBytes(snapshot.system.memory_used_bytes)}</strong></div>
      <div><span>Total</span><strong>{formatBytes(snapshot.system.memory_total_bytes)}</strong></div>
      <div><span>Swap</span><strong>{formatPercent(swapPercent)}</strong></div>
      <div>
        {#if snapshot.system.memory_available_bytes !== undefined}
          <span>Available</span><strong>{formatBytes(snapshot.system.memory_available_bytes)}</strong>
        {:else}
          <span>Processes</span><strong>{snapshot.system.process_count}</strong>
        {/if}
      </div>
      {#if memoryAccounting}
        <div><span>Process WS</span><strong>{formatBytes(memoryAccounting.process_working_set_bytes)}</strong></div>
        <div><span>Process private</span><strong>{formatBytes(memoryAccounting.process_private_bytes)}</strong></div>
        <div><span>Blocked rows</span><strong>{memoryAccounting.denied_process_count}</strong></div>
        <div><span>Unattributed</span><strong>{optionalBytes(memoryAccounting.unattributed_bytes)}</strong></div>
        <div>
          <span>Commit</span>
          <strong>
            {memoryAccounting.commit_used_bytes === undefined
              ? "--"
              : `${formatBytes(memoryAccounting.commit_used_bytes)} / ${optionalBytes(memoryAccounting.commit_limit_bytes)}`}
          </strong>
        </div>
        <div><span>Kernel paged</span><strong>{optionalBytes(memoryAccounting.kernel_paged_pool_bytes)}</strong></div>
        <div><span>Kernel nonpaged</span><strong>{optionalBytes(memoryAccounting.kernel_nonpaged_pool_bytes)}</strong></div>
        <div><span>System cache</span><strong>{optionalBytes(memoryAccounting.system_cache_bytes)}</strong></div>
      {/if}
    </div>
    {#if topKernelPoolTags.length > 0}
      <section class="pool-tag-panel" aria-label="Top kernel pool tags">
        <div class="pool-tag-heading">
          <div>
            <span>Top kernel pool tags</span>
            <small>Pool tags identify kernel allocation categories; driver matches are best-effort candidates.</small>
          </div>
          <strong>{topKernelPoolTags.length}</strong>
        </div>
        <div class="pool-tag-list">
          {#each topKernelPoolTags as tag (poolTagKey(tag))}
            <div class="pool-tag-row">
              <span><b>{tag.tag}</b><small>{poolKindLabel(tag.kind)}</small></span>
              <strong>{formatBytes(tag.bytes)}</strong>
              <span class="pool-tag-counts">{tag.allocations} alloc / {tag.frees} free</span>
              <span class="pool-tag-candidates">{driverCandidateLabel(tag)}</span>
            </div>
          {/each}
        </div>
      </section>
    {/if}
    <div class="detail-chart-grid two-up">
      <div class="detail-chart-card large">
        <div><span>Memory load</span><strong>{formatPercent(memoryPercent)}</strong></div>
        <MiniChart values={history.memory} max={100} stroke={activeTheme.memoryStroke} fill={activeTheme.memoryFill} />
      </div>
      <div class="detail-chart-card large">
        <div><span>Swap load</span><strong>{formatPercent(swapPercent)}</strong></div>
        <MiniChart values={history.swap} max={100} stroke={activeTheme.swapStroke} fill={activeTheme.swapFill} />
      </div>
    </div>
  {:else if detailMode === "disk"}
    <div class="detail-summary" aria-label="Disk summary">
      <div><span>Read rate</span><strong>{formatRate(diskReadRate)}</strong></div>
      <div><span>Write rate</span><strong>{formatRate(diskWriteRate)}</strong></div>
      <div><span>Read total</span><strong>{formatBytes(snapshot.system.disk_read_total_bytes)}</strong></div>
      <div><span>Write total</span><strong>{formatBytes(snapshot.system.disk_write_total_bytes)}</strong></div>
    </div>
    <div class="detail-chart-grid two-up">
      <div class="detail-chart-card large">
        <div><span>Read throughput</span><strong>{formatRate(diskReadRate)}</strong></div>
        <MiniChart values={history.diskRead} max={diskScaleMax} stroke={activeTheme.diskReadStroke} fill={activeTheme.diskReadFill} />
      </div>
      <div class="detail-chart-card large">
        <div><span>Write throughput</span><strong>{formatRate(diskWriteRate)}</strong></div>
        <MiniChart values={history.diskWrite} max={diskScaleMax} stroke={activeTheme.diskWriteStroke} fill={activeTheme.diskWriteFill} />
      </div>
    </div>
  {:else}
    <div class="detail-summary" aria-label="Network summary">
      <div><span>Down</span><strong>{formatRate(networkDownRate)}</strong></div>
      <div><span>Up</span><strong>{formatRate(networkUpRate)}</strong></div>
      <div><span>Received</span><strong>{formatBytes(snapshot.system.network_received_total_bytes)}</strong></div>
      <div><span>Sent</span><strong>{formatBytes(snapshot.system.network_transmitted_total_bytes)}</strong></div>
      <div><span>Source</span><strong>{metricQualityLabel(systemQuality.network, "Aggregate")}</strong></div>
    </div>
    <div class="detail-chart-grid two-up">
      <div class="detail-chart-card large">
        <div><span>Download rate</span><strong>{formatRate(networkDownRate)}</strong></div>
        <MiniChart values={history.netRx} max={networkScaleMax} stroke={activeTheme.networkDownStroke} fill={activeTheme.networkDownFill} />
      </div>
      <div class="detail-chart-card large">
        <div><span>Upload rate</span><strong>{formatRate(networkUpRate)}</strong></div>
        <MiniChart values={history.netTx} max={networkScaleMax} stroke={activeTheme.networkUpStroke} fill={activeTheme.networkUpFill} />
      </div>
    </div>
  {/if}
</section>
