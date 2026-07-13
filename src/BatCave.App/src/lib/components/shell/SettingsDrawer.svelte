<script lang="ts">
  import { X } from "phosphor-svelte";
  import type { PrivilegedCollectionAction } from "../../environmentPresentation";
  import { focusDialogStart, trapDialogFocus } from "../../dialogFocus";
  import { formatInterval } from "../../format";
  import { platformPresentation, type PlatformPresentation } from "../../platformPresentation";
  import type { ThemeOption, ThemePreference } from "../../themes";

  export let open = false;
  export let themeOptions: ThemeOption[];
  export let themePreference: ThemePreference;
  export let pollIntervals: readonly number[];
  export let pollIntervalMs: number;
  export let historyPointOptions: readonly number[];
  export let historyPointLimit: number;
  export let adminAvailable = true;
  export let runtimeMutationsDisabled = false;
  export let processStatus = "Standard token";
  export let adminStatus = "Off";
  export let adminNote = "Protected fields remain unavailable until the local helper is enabled.";
  export let adminAction: PrivilegedCollectionAction | null = null;
  export let dataDirectory: string | null = null;
  export let presentation: PlatformPresentation = platformPresentation({ platform: "fixture" });
  export let updateStatus: "idle" | "checking" | "available" | "current" | "installing" | "error" = "idle";
  export let updateMessage = "Checks only when you ask.";
  export let onClose: () => void = () => {};
  export let onTheme: (theme: ThemePreference) => void;
  export let onPollInterval: (interval: number) => void;
  export let onHistoryLimit: (limit: number) => void;
  export let onAdminMode: (enabled: boolean) => void = () => {};
  export let onCheckForUpdates: () => void = () => {};
  export let onInstallUpdate: () => void = () => {};
  export let onResetHistory: () => void = () => {};

  let resetConfirm = false;
  let dialog: HTMLDialogElement | null = null;
  let opener: HTMLElement | null = null;

  $: if (dialog) syncDialog(dialog, open);

  function syncDialog(element: HTMLDialogElement, shouldOpen: boolean): void {
    if (shouldOpen && !element.open) {
      opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      element.showModal();
      focusDialogStart(element);
    } else if (!shouldOpen && element.open) {
      resetConfirm = false;
      element.close();
      restoreOpener();
    }
  }

  function requestClose(): void {
    resetConfirm = false;
    dialog?.close();
    restoreOpener();
    onClose();
  }

  function handleClose(): void {
    resetConfirm = false;
    restoreOpener();
  }

  function restoreOpener(): void {
    opener?.focus();
    opener = null;
  }

  function handleBackdropClick(event: MouseEvent): void {
    if (event.target === event.currentTarget) requestClose();
  }

  function requestReset(): void {
    if (!resetConfirm) {
      resetConfirm = true;
      return;
    }

    onResetHistory();
    resetConfirm = false;
  }
</script>

<dialog
  bind:this={dialog}
  class="drawer-layer"
  aria-labelledby="settings-title"
  tabindex="-1"
  oncancel={(event) => {
    event.preventDefault();
    requestClose();
  }}
  onclose={handleClose}
  onkeydown={(event) => trapDialogFocus(event, dialog)}
  onclick={handleBackdropClick}
>
    <div class="settings-drawer">
      <header class="drawer-header">
        <div>
          <span>Preferences</span>
          <h2 id="settings-title">Settings</h2>
        </div>
        <button
          class="icon-action"
          type="button"
          aria-label="Close settings"
          data-dialog-initial-focus
          onclick={requestClose}
        >
          <X size={20} weight="bold" aria-hidden="true" />
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
            <select disabled={runtimeMutationsDisabled} value={pollIntervalMs} onchange={(event) => onPollInterval(Number(event.currentTarget.value))}>
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

        {#if adminAvailable}
          <section class="settings-section privileged-section">
            <div class="settings-section-heading">
              <h3>{presentation.privilegedAccessLabel}</h3>
              <p>{presentation.privilegedAccessDescription}</p>
            </div>
            <div class="privileged-card">
              <div>
                <strong>Current process</strong>
                <span>{processStatus}</span>
              </div>
            </div>
            <div class="privileged-card">
              <div>
                <strong>Privileged collection</strong>
                <span>{adminStatus}</span>
              </div>
              {#if adminAction}
                <button type="button" disabled={runtimeMutationsDisabled} onclick={() => onAdminMode(adminAction.enabled)}>
                  {adminAction.label}
                </button>
              {/if}
            </div>
            <p class="setting-note">{adminNote}</p>
          </section>
        {/if}

        <section class="settings-section">
          <div class="settings-section-heading">
            <h3>Updates</h3>
            <p>BatCave checks the stable GitHub release channel only when you ask.</p>
          </div>
          <div class="privileged-card">
            <div>
              <strong>Stable channel</strong>
              <span aria-live="polite">{updateMessage}</span>
            </div>
            <button
              type="button"
              disabled={updateStatus === "checking" || updateStatus === "installing"}
              onclick={updateStatus === "available" ? onInstallUpdate : onCheckForUpdates}
            >
              {updateStatus === "checking"
                ? "Checking…"
                : updateStatus === "installing"
                  ? "Installing…"
                  : updateStatus === "available"
                    ? "Download and install"
                    : updateStatus === "error"
                      ? "Retry"
                      : "Check now"}
            </button>
          </div>
          <p class="setting-note">
            Checks contact github.com. Prereleases and downgrades are never installed automatically; invalid signatures are rejected.
          </p>
        </section>

        <section class="settings-section">
          <div class="settings-section-heading">
            <h3>Local data</h3>
            <p>Runtime state stays on this machine.</p>
            <code class="data-directory">{dataDirectory || "Not available"}</code>
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
</dialog>
