<script lang="ts">
  import ArrowClockwise from "phosphor-svelte/lib/ArrowClockwise";
  import Pause from "phosphor-svelte/lib/Pause";
  import Play from "phosphor-svelte/lib/Play";
  import X from "phosphor-svelte/lib/X";
  import { focusDialogStart, trapDialogFocus } from "../../dialogFocus";
  import { formatBytes, formatInterval } from "../../format";
  import {
    defaultNarrativeCapability,
    narrativeCapabilityExplanation,
    type NarrativeCapability,
  } from "../../narratives";
  import { platformPresentation, type PlatformPresentation } from "../../platformPresentation";
  import type {
    ThemeFamily,
    ThemeFamilyOption,
    ThemeModeOption,
    ThemeModePreference,
    ThemePreference,
  } from "../../themes";

  export let open = false;
  export let themeFamilyOptions: ThemeFamilyOption[];
  export let themeModeOptions: ThemeModeOption[];
  export let themePreference: ThemePreference;
  export let pollIntervals: readonly number[];
  export let pollIntervalMs: number;
  export let historyPointOptions: readonly number[];
  export let historyPointLimit: number;
  export let isPaused = false;
  export let commandError = "";
  export let adminAvailable = true;
  export let runtimeMutationsDisabled = false;
  export let processStatus = "Standard token";
  export let adminStatus = "Off";
  export let adminNote =
    "Protected fields require the installed collector service or an elevated current process.";
  export let dataDirectory: string | null = null;
  export let presentation: PlatformPresentation = platformPresentation({ platform: "fixture" });
  export let updateStatus:
    | "idle"
    | "checking"
    | "available"
    | "current"
    | "installing"
    | "error" = "idle";
  export let updateMessage = "Checks only when you ask.";
  export let enhancedNarratives = false;
  export let narrativeCapability: NarrativeCapability = defaultNarrativeCapability;
  export let narrativeSettingsStatus = "";
  export let narrativeModelAction: "idle" | "downloading" | "cancelling" = "idle";
  export let onClose: () => void = () => {};
  export let onThemeFamily: (family: ThemeFamily) => void;
  export let onThemeMode: (mode: ThemeModePreference) => void;
  export let onEnhancedNarratives: (enabled: boolean) => void = () => {};
  export let onDownloadNarrativeModel: () => void = () => {};
  export let onCancelNarrativeModelDownload: () => void = () => {};
  export let onPollInterval: (interval: number) => void;
  export let onHistoryLimit: (limit: number) => void;
  export let onPaused: () => void = () => {};
  export let onRefresh: () => void = () => {};
  export let onOpenDiagnostics: () => void = () => {};
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

  function narrativeProviderLabel(capability: NarrativeCapability): string {
    if (capability.provider === "apple_foundation") return "Apple Intelligence on this Mac";
    if (capability.provider === "foundry_local") return "Foundry Local";
    return "Local model unavailable";
  }

  function narrativeDownloadProgress(capability: NarrativeCapability): number {
    if (!capability.download_size_bytes) return 0;
    return Math.min(
      100,
      Math.max(0, ((capability.downloaded_bytes ?? 0) / capability.download_size_bytes) * 100),
    );
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
          <p>Choose a color family and how it follows the system.</p>
        </div>
        <div class="theme-family-grid" aria-label="Theme family">
          {#each themeFamilyOptions as option}
            <button
              class:active={themePreference.family === option.family}
              type="button"
              aria-label={option.ariaLabel}
              aria-pressed={themePreference.family === option.family}
              onclick={() => onThemeFamily(option.family)}
            >
              <span class={`theme-family-swatch theme-family-${option.family}`} aria-hidden="true"></span>
              <span>{option.label}</span>
            </button>
          {/each}
        </div>
        <div class="segmented theme-mode-options" role="group" aria-label="Appearance mode">
          {#each themeModeOptions as option}
            <button
              class:active={themePreference.mode === option.mode}
              type="button"
              aria-label={option.ariaLabel}
              aria-pressed={themePreference.mode === option.mode}
              onclick={() => onThemeMode(option.mode)}
            >{option.label}</button>
          {/each}
        </div>
      </section>

      <section class="settings-section narrative-settings">
        <div class="settings-section-heading">
          <h3>Enhanced explanations</h3>
          <p>Optionally rewrites two short explanations with a model running on this machine.</p>
        </div>
        <label class="setting-row narrative-toggle">
          <span>
            <strong>Use locally generated explanations</strong>
            <small>Off by default. Deterministic explanations always remain available.</small>
          </span>
          <input
            type="checkbox"
            role="switch"
            checked={enhancedNarratives}
            onchange={(event) => onEnhancedNarratives(event.currentTarget.checked)}
          />
        </label>
        <p class="setting-note">
          Turning this on never downloads a model. Only rounded workload facts are shared with the local provider; paths, process IDs, diagnostics, and other workloads are excluded.
        </p>
        <div class="narrative-provider-card">
          <div>
            <strong>{narrativeProviderLabel(narrativeCapability)}</strong>
            <span>{narrativeCapabilityExplanation(narrativeCapability)}</span>
          </div>
          {#if narrativeCapability.model_name || narrativeCapability.download_size_bytes}
            <dl>
              {#if narrativeCapability.model_name}<div><dt>Model</dt><dd>{narrativeCapability.model_name}</dd></div>{/if}
              {#if narrativeCapability.download_size_bytes !== undefined}<div><dt>Download</dt><dd>{formatBytes(narrativeCapability.download_size_bytes)}</dd></div>{/if}
              {#if narrativeCapability.license_name}
                <div>
                  <dt>License</dt>
                  <dd title={narrativeCapability.license_url}>{narrativeCapability.license_name}</dd>
                </div>
              {/if}
            </dl>
          {/if}
          {#if narrativeModelAction === "cancelling"}
            <button type="button" disabled>Cancelling…</button>
          {:else if narrativeModelAction === "downloading" || narrativeCapability.download_state === "downloading"}
            <progress
              aria-label="Local model download progress"
              max="100"
              value={narrativeDownloadProgress(narrativeCapability)}
            ></progress>
            <button type="button" disabled={!narrativeCapability.can_cancel_download} onclick={onCancelNarrativeModelDownload}>Cancel download</button>
          {:else if narrativeCapability.can_download}
            <button type="button" onclick={onDownloadNarrativeModel}>Download local model</button>
          {/if}
        </div>
        {#if narrativeSettingsStatus}
          <p class="setting-note narrative-settings-status" role="status" aria-live="polite">
            {narrativeSettingsStatus}
          </p>
        {/if}
      </section>

      <section class="settings-section">
        <div class="settings-section-heading">
          <h3>Advanced sampling and chart history</h3>
          <p>Adjust collection only when you need a different balance of detail and overhead.</p>
        </div>
        <div class="sampling-actions">
          <button type="button" disabled={runtimeMutationsDisabled} onclick={onPaused}>
            {#if isPaused}<Play size={17} weight="fill" aria-hidden="true" />{:else}<Pause size={17} weight="fill" aria-hidden="true" />{/if}
            {isPaused ? "Resume monitoring" : "Pause monitoring"}
          </button>
          <button type="button" onclick={onRefresh}>
            <ArrowClockwise size={17} weight="bold" aria-hidden="true" />
            Refresh now
          </button>
        </div>
        <label class="setting-row">
          <span>Refresh cadence</span>
          <select
            disabled={runtimeMutationsDisabled}
            value={pollIntervalMs}
            onchange={(event) => onPollInterval(Number(event.currentTarget.value))}
          >
            {#each pollIntervals as interval}
              <option value={interval}>{formatInterval(interval)}</option>
            {/each}
          </select>
        </label>
        <label class="setting-row">
          <span>Chart history</span>
          <select
            value={historyPointLimit}
            onchange={(event) => onHistoryLimit(Number(event.currentTarget.value))}
          >
            {#each historyPointOptions as option}
              <option value={option}>{option} samples</option>
            {/each}
          </select>
        </label>
        <div class="history-reset-actions">
          <button class="danger-outline" type="button" onclick={requestReset}>
            {resetConfirm ? "Confirm reset history" : "Reset chart history"}
          </button>
          {#if resetConfirm}
            <button class="text-action" type="button" onclick={() => (resetConfirm = false)}>Cancel</button>
          {/if}
        </div>
        {#if commandError}
          <p class="command-error" role="alert">{commandError}</p>
        {/if}
      </section>

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
          Checks contact github.com. Prereleases and downgrades are not offered; downloaded updates must pass signature verification.
        </p>
      </section>

      <section class="settings-section">
        <div class="settings-section-heading">
          <h3>Local data and diagnostics</h3>
          <p>Runtime state stays on this machine. Exact collector evidence is available on demand.</p>
        </div>
        <div class="local-data-summary">
          <span>Data directory</span>
          <code class="data-directory">{dataDirectory || "Not available"}</code>
        </div>
        {#if adminAvailable}
          <details class="technical-disclosure settings-technical">
            <summary>{presentation.privilegedAccessLabel}</summary>
            <div class="privileged-card">
              <div><strong>Current process</strong><span>{processStatus}</span></div>
            </div>
            <div class="privileged-card">
              <div><strong>Privileged collection</strong><span>{adminStatus}</span></div>
            </div>
            <p class="setting-note">{adminNote}</p>
          </details>
        {/if}
        <button class="diagnostics-action" type="button" onclick={onOpenDiagnostics}>
          Open diagnostics
        </button>
      </section>
    </div>
  </div>
</dialog>
