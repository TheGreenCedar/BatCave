<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import uPlot from "uplot";

  export let values: number[] = [];
  export let min = 0;
  export let max = 100;
  export let stroke = "#5eead4";
  export let fill = "rgba(94, 234, 212, 0.12)";

  let host: HTMLDivElement | undefined = undefined;
  let chart: uPlot | undefined;
  let resizeObserver: ResizeObserver | undefined;
  let resizeFrame = 0;
  let lastWidth = 0;
  let lastHeight = 0;
  const minChartHeight = 28;

  $: if (chart) {
    chart.setData(makeData(values));
    chart.setScale("y", { min, max });
  }

  onMount(() => {
    if (!host) {
      return;
    }

    const bounds = host.getBoundingClientRect();
    chart = new uPlot(makeOptions(bounds.width, bounds.height), makeData(values), host);
    resizeObserver = new ResizeObserver(() => scheduleResize());
    resizeObserver.observe(host);
    resize();
  });

  onDestroy(() => {
    if (resizeFrame !== 0) {
      window.cancelAnimationFrame(resizeFrame);
    }

    resizeObserver?.disconnect();
    chart?.destroy();
  });

  function scheduleResize(): void {
    if (resizeFrame !== 0) {
      return;
    }

    resizeFrame = window.requestAnimationFrame(() => {
      resizeFrame = 0;
      resize();
    });
  }

  function resize(): void {
    const node = host;
    if (!chart || !node) {
      return;
    }

    const bounds = node.getBoundingClientRect();
    const width = Math.max(80, Math.floor(bounds.width));
    const height = Math.max(minChartHeight, Math.floor(bounds.height));

    if (width === lastWidth && height === lastHeight) {
      return;
    }

    lastWidth = width;
    lastHeight = height;
    chart.setSize({ width, height });
  }

  function makeData(series: number[]): uPlot.AlignedData {
    const points = series.length > 1 ? series : [0, 0];
    const x = points.map((_, index) => index);
    const y = series.length > 1 ? points : [0, points[0] ?? 0];
    return [x, y];
  }

  function makeOptions(width: number, height: number): uPlot.Options {
    return {
      width: Math.max(80, Math.floor(width)),
      height: Math.max(minChartHeight, Math.floor(height)),
      padding: [0, 0, 0, 0],
      scales: {
        x: { time: false },
        y: { range: () => [min, max] },
      },
      axes: [{ show: false }, { show: false }],
      series: [{}, { stroke, fill, width: 2, points: { show: false } }],
      legend: { show: false },
      cursor: {
        show: false,
        drag: { setScale: false },
      },
    };
  }
</script>

<div class="chart-host" bind:this={host} aria-hidden="true"></div>
