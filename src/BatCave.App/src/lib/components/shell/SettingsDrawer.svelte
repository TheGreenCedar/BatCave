<script lang="ts">
  import { tick } from "svelte";
  import { formatInterval } from "../../format";
  import type { ThemeOption, ThemePreference } from "../../themes";

  export let open = false;
  export let themeOptions: ThemeOption[];
  export let themePreference: ThemePreference;
  export let pollIntervals: readonly number[];
  export let pollIntervalMs: number;
  export let historyPointOptions: readonly number[];
  export let historyPointLimit: number;
  export let adminRequested: boolean;
  export let adminEnabled: boolean;
  export let onClose: () => void;
  export let onTheme: (theme: ThemePreference) => void;
  export let onPollInterval: (interval: number) => void;
  export let onHistoryLimit: (limit: number) => void;
  export let onAdminMode: (enabled: boolean) => void;
  export let onResetHistory: () => void = () => {};

  let resetConfirm = false;
  let closeButton: HTMLButtonElement | null = null;
  let previouslyOpen = false;

  $: if (open && !previouslyOpen) {
    previouslyOpen = true;
    void tick().then(() => closeButton?.focus());
  }
  $: if (!open) previouslyOpen = false;

  function requestReset(): void {
    if (!resetConfirm) {
      resetConfirm = true;
      return;
    }

    onResetHistory();
    resetConfirm = false;
  }
</script>

<svelte:window
  onkeydown={(event) => {
    if (open && event.key === "Escape") onClose();
  }}
/>

{#if open}
  <div class="drawer-layer">
    <button class="drawer-backdrop" type="button" aria-label="Close settings" onclick={onClose}></button>
    <div class="settings-drawer" role="dialog" aria-modal="true" aria-labelledby="settings-title">
      <header class="drawer-header">
        <div>
          <span>Preferences</span>
          <h2 id="settings-title">Settings</h2>
        </div>
        <button bind:this={closeButton} class="icon-action" type="button" aria-label="Close settings" onclick={onClose}>
          <svg class="control-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18" /></svg>
        </button>
      </header>

      <div class="drawer-scroll">
        <section class="settings-section">
          <div class="settings-section-heading">
            <h3>Appearance</h3>
            <p>Choose the palette that keeps the telemetry readable for you.</p>
          </div>
          <div class="choice-grid">
            {#each themeOptions as theme}
              <button
                class:active={themePreference === theme.name}
                type="button"
                aria-label={theme.ariaLabel}
                aria-pressed={themePreference === theme.name}
                onclick={() => onTheme(theme.name)}
              >
                {theme.label}
              </button>
            {/each}
          </div>
        </section>

        <section class="settings-section">
          <div class="settings-section-heading">
            <h3>Sampling</h3>
            <p>Faster sampling reacts sooner but costs more local CPU.</p>
          </div>
          <label class="setting-row">
            <span>Refresh cadence</span>
            <select value={pollIntervalMs} onchange={(event) => onPollInterval(Number(event.currentTarget.value))}>
              {#each pollIntervals as interval}
                <option value={interval}>{formatInterval(interval)}</option>
              {/each}
            </select>
          </label>
          <label class="setting-row">
            <span>History window</span>
            <select value={historyPointLimit} onchange={(event) => onHistoryLimit(Number(event.currentTarget.value))}>
              {#each historyPointOptions as option}
                <option value={option}>{option} samples</option>
              {/each}
            </select>
          </label>
        </section>

        <section class="settings-section privileged-section">
          <div class="settings-section-heading">
            <h3>Privileged access</h3>
            <p>Admin mode can fill permission-shaped gaps. BatCave still falls back safely if elevation is denied.</p>
          </div>
          <div class="privileged-card">
            <div>
              <strong>Admin mode</strong>
              <span>{adminEnabled ? "Active" : adminRequested ? "Waiting for Windows" : "Off"}</span>
            </div>
            <button
              class:active={adminRequested}
              type="button"
              aria-pressed={adminRequested}
              onclick={() => onAdminMode(!adminRequested)}
            >
              {adminRequested ? "Disable" : "Enable"}
            </button>
          </div>
          <p class="setting-note">Enabling this may open a Windows elevation prompt. Denying it leaves standard monitoring active.</p>
        </section>

        <section class="settings-section">
          <div class="settings-section-heading">
            <h3>Local data</h3>
            <p>Runtime state stays on this machine under %LOCALAPPDATA%\BatCaveMonitor.</p>
          </div>
          <button class="danger-outline" type="button" onclick={requestReset}>
            {resetConfirm ? "Confirm reset history" : "Reset chart history"}
          </button>
          {#if resetConfirm}
            <button class="text-action" type="button" onclick={() => (resetConfirm = false)}>Cancel</button>
          {/if}
        </section>
      </div>
    </div>
  </div>
{/if}
