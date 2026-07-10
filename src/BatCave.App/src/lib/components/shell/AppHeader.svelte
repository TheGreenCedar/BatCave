<script lang="ts">
  export let isPaused = false;
  export let pollState: "starting" | "native" | "fixture" | "error" = "starting";
  export let updatedAtLabel: string;
  export let healthLabel: string;
  export let healthTone: "healthy" | "warning" | "danger";
  export let onOpenDiagnostics: () => void;

  $: runtimeLabel =
    pollState === "error"
      ? "Stale"
      : isPaused
        ? "Paused"
        : pollState === "native"
          ? "Live"
          : pollState === "fixture"
            ? "Fixture data"
            : "Starting";
</script>

<header class="app-header">
  <div class="brand-lockup">
    <span class="brand-mark" aria-hidden="true">
      <svg viewBox="0 0 48 28">
        <path d="M4 11 L13 5 L19 9 L24 3 L29 9 L35 5 L44 11 L37 18 L31 15 L24 24 L17 15 L11 18 Z" />
      </svg>
    </span>
    <div>
      <h1>BatCave</h1>
      <p>Local resource triage</p>
    </div>
  </div>

  <div class="header-status" aria-label="Runtime status">
    <span class="runtime-state" class:paused={isPaused} class:error={pollState === "error"}>
      <i aria-hidden="true"></i>
      {runtimeLabel}
    </span>
    <span class="sample-age">Sampled {updatedAtLabel}</span>
    <button
      class="health-chip"
      class:warning={healthTone === "warning"}
      class:danger={healthTone === "danger"}
      type="button"
      onclick={onOpenDiagnostics}
    >{healthLabel}</button>
  </div>
</header>
