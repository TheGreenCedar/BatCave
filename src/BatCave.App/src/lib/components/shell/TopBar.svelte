<script lang="ts">
  export let isPaused: boolean;
  export let pollState: "starting" | "native" | "fixture" | "error";
  export let processCount: number;
  export let latencyMs: number;
  export let updatedAtLabel: string;
</script>

<header class="topbar">
  <div class="brand-lockup">
    <span class="brand-mark" aria-hidden="true">
      <svg viewBox="0 0 48 28">
        <path d="M4 11 L13 5 L19 9 L24 3 L29 9 L35 5 L44 11 L37 18 L31 15 L24 24 L17 15 L11 18 Z" />
      </svg>
    </span>
    <div>
      <h1>BatCave Monitor</h1>
      <p>v0.1.0</p>
    </div>
  </div>
  <div class="status-stack" aria-label="Runtime status">
    <span class:live={pollState === "native"} class:paused={isPaused} class:fixture={pollState === "fixture"}>
      <i aria-hidden="true"></i>{isPaused ? "Paused" : pollState === "native" ? "Live" : pollState === "fixture" ? "Fixture data" : pollState}
    </span>
    <span>Processes <b>{processCount}</b></span>
    <span>Updated <b>{updatedAtLabel}</b></span>
    <span>Collector <b>{latencyMs} ms</b></span>
  </div>
</header>
