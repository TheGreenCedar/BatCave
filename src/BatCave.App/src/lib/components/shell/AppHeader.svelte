<script lang="ts">
  import {
    ArrowClockwise,
    CaretDown,
    Circle,
    GearSix,
    MagnifyingGlass,
    Pause,
    Play,
  } from "phosphor-svelte";

  export let searchText = "";
  export let isPaused = false;
  export let pollState: "starting" | "native" | "fixture" | "error" = "starting";
  export let healthLabel: string;
  export let healthTone: "healthy" | "warning" | "danger";
  export let mutationsDisabled = false;
  export let onSearch: (value: string) => void;
  export let onPaused: () => void;
  export let onRefresh: () => void;
  export let onOpenSettings: () => void;
  export let onOpenDiagnostics: () => void;

  const brandIconUrl = new URL("../../../../src-tauri/icons/64x64.png", import.meta.url).href;
</script>

<header class="app-header">
  <div class="brand-lockup">
    <img class="brand-mark" src={brandIconUrl} alt="" />
    <div>
      <h1>BatCave</h1>
      <p>Local resource triage</p>
    </div>
  </div>

  <label class="header-search" for="process-search">
    <MagnifyingGlass size={19} weight="regular" aria-hidden="true" />
    <input
      id="process-search"
      value={searchText}
      oninput={(event) => onSearch(event.currentTarget.value)}
      aria-label="Search apps and processes"
      placeholder="Search apps and processes"
      autocomplete="off"
      disabled={mutationsDisabled}
    />
    <kbd>/</kbd>
  </label>

  <nav class="header-actions" aria-label="Telemetry controls">
    <button class="header-action" class:resume={isPaused} type="button" disabled={mutationsDisabled} onclick={onPaused}>
      {#if isPaused}<Play size={18} weight="fill" aria-hidden="true" />{:else}<Pause size={18} weight="fill" aria-hidden="true" />{/if}
      <span>{isPaused ? "Resume" : "Pause"}</span>
    </button>
    <button class="header-action" type="button" onclick={onRefresh}>
      <ArrowClockwise size={18} weight="bold" aria-hidden="true" />
      <span>Refresh</span>
    </button>
    <button class="header-action" type="button" onclick={onOpenSettings}>
      <GearSix size={19} weight="regular" aria-hidden="true" />
      <span>Settings</span>
    </button>
  </nav>

  <button
    class="health-chip"
    class:warning={healthTone === "warning"}
    class:danger={healthTone === "danger"}
    type="button"
    aria-label={`${healthLabel}. Open diagnostics.`}
    onclick={onOpenDiagnostics}
  >
    <Circle size={10} weight="fill" aria-hidden="true" />
    <span>{pollState === "fixture" ? "Fixture" : healthLabel}</span>
    <CaretDown size={14} weight="bold" aria-hidden="true" />
  </button>
</header>
