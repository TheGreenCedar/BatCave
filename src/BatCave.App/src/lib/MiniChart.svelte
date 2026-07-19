<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import uPlot from "uplot";
  import {
    createChartMotion,
    shouldSnapChartMotion,
    type ChartMotion,
  } from "./chartMotion";

  export let values: number[] = [];
  export let min = 0;
  export let max = 100;
  export let stroke = "#5eead4";
  export let fill = "rgba(94, 234, 212, 0.12)";

  let host: HTMLDivElement | undefined = undefined;
  let chart: uPlot | undefined;
  let chartMotion: ChartMotion | undefined;
  let reducedMotionQuery: MediaQueryList | undefined;
  let resizeObserver: ResizeObserver | undefined;
  let resizeFrame = 0;
  let lastWidth = 0;
  let lastHeight = 0;
  let appliedStroke = stroke;
  let appliedFill = fill;
  const minChartHeight = 28;

  $: if (chart && chartMotion) {
    chartMotion.update(values, { snap: shouldSnapMotion() });
    chart.setScale("y", { min, max });
  }

  $: if (chart && (stroke !== appliedStroke || fill !== appliedFill)) {
    appliedStroke = stroke;
    appliedFill = fill;
    chart.series[1].stroke = () => stroke;
    chart.series[1].fill = () => fill;
    chart.redraw(false, false);
  }

  onMount(() => {
    if (!host) {
      return;
    }

    const bounds = host.getBoundingClientRect();
    appliedStroke = stroke;
    appliedFill = fill;
    const initialValues = [...values];
    chart = new uPlot(makeOptions(bounds.width, bounds.height), makeData(initialValues), host);
    chartMotion = createChartMotion(
      initialValues,
      (nextValues) => chart?.setData(makeData(nextValues)),
      {
        now: () => performance.now(),
        request: (callback) => window.requestAnimationFrame(callback),
        cancel: (frame) => window.cancelAnimationFrame(frame),
      },
    );
    reducedMotionQuery = window.matchMedia("(prefers-reduced-motion: reduce)");
    reducedMotionQuery.addEventListener("change", handleMotionPreferenceChange);
    document.addEventListener("visibilitychange", handleVisibilityChange);
    resizeObserver = new ResizeObserver(() => scheduleResize());
    resizeObserver.observe(host);
    resize();
  });

  onDestroy(() => {
    chartMotion?.destroy();
    reducedMotionQuery?.removeEventListener("change", handleMotionPreferenceChange);
    document.removeEventListener("visibilitychange", handleVisibilityChange);

    if (resizeFrame !== 0) {
      window.cancelAnimationFrame(resizeFrame);
    }

    resizeObserver?.disconnect();
    chart?.destroy();
    chart = undefined;
  });

  function shouldSnapMotion(): boolean {
    return shouldSnapChartMotion(
      document.visibilityState,
      reducedMotionQuery?.matches ?? false,
    );
  }

  function handleMotionPreferenceChange(event: MediaQueryListEvent): void {
    if (event.matches) chartMotion?.finish();
  }

  function handleVisibilityChange(): void {
    if (document.visibilityState !== "visible") chartMotion?.finish();
  }

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
