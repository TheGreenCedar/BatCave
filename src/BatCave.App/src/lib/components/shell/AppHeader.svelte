<script lang="ts">
  import Circle from "phosphor-svelte/lib/Circle";
  import GearSix from "phosphor-svelte/lib/GearSix";
  import brandIcon from "../../../../src-tauri/icons/64x64.png";

  type AppView = "overview" | "explore";

  export let activeView: AppView = "overview";
  export let pollState: "starting" | "native" | "fixture" | "error" = "starting";
  export let healthLabel: string;
  export let healthTone: "healthy" | "warning" | "danger";
  export let onNavigate: (view: AppView) => void;
  export let onOpenSettings: () => void;
  export let onOpenDiagnostics: () => void;
</script>

<header class="app-header">
  <div class="brand-lockup">
    <img class="brand-mark" src={brandIcon} alt="" />
    <div>
      <h1>BatCave</h1>
      <p>Local resource monitor</p>
    </div>
  </div>

  <nav class="view-navigation" aria-label="Primary navigation">
    <button
      type="button"
      data-view="overview"
      class:active={activeView === "overview"}
      aria-current={activeView === "overview" ? "page" : undefined}
      onclick={() => onNavigate("overview")}
    >Overview</button>
    <button
      type="button"
      data-view="explore"
      class:active={activeView === "explore"}
      aria-current={activeView === "explore" ? "page" : undefined}
      onclick={() => onNavigate("explore")}
    >Explore</button>
  </nav>

  <div class="header-status-actions">
    {#if healthTone === "healthy" && pollState !== "fixture"}
      <span class="monitoring-status" aria-label="Monitoring is active">
        <Circle size={10} weight="fill" aria-hidden="true" />
        Monitoring
      </span>
    {:else}
      <button
        class="health-chip"
        class:warning={healthTone === "warning"}
        class:danger={healthTone === "danger"}
        type="button"
        aria-label={`${pollState === "fixture" ? "Layout fixture" : healthLabel}. Open diagnostics.`}
        onclick={onOpenDiagnostics}
      >
        <Circle size={10} weight="fill" aria-hidden="true" />
        <span>{pollState === "fixture" ? "Layout fixture" : healthLabel}</span>
      </button>
    {/if}
    <span class="header-divider" aria-hidden="true"></span>
    <button class="settings-action" type="button" onclick={onOpenSettings}>
      <GearSix size={20} weight="regular" aria-hidden="true" />
      <span>Settings</span>
    </button>
  </div>
</header>
