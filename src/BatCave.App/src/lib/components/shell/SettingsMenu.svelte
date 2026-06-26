<script lang="ts">
  import { formatInterval } from "../../format";
  import type { ThemeOption, ThemePreference } from "../../themes";

  export let themeOptions: ThemeOption[];
  export let themePreference: ThemePreference;
  export let pollIntervals: readonly number[];
  export let pollIntervalMs: number;
  export let historyPointOptions: readonly number[];
  export let historyPointLimit: number;
  export let adminRequested: boolean;
  export let adminEnabled: boolean;
  export let onTheme: (theme: ThemePreference) => void;
  export let onPollInterval: (interval: number) => void;
  export let onHistoryLimit: (limit: number) => void;
  export let onAdminMode: (enabled: boolean) => void;
  export let onResetHistory: () => void;
</script>

<details class="settings-menu">
  <summary aria-label="Open settings">
    <svg class="control-icon filled" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M10.8 3h2.4l.5 2.3c.6.2 1.1.4 1.6.7l2-1.2 1.7 1.7-1.2 2c.3.5.5 1 .7 1.6l2.3.5v2.4l-2.3.5c-.2.6-.4 1.1-.7 1.6l1.2 2-1.7 1.7-2-1.2c-.5.3-1 .5-1.6.7l-.5 2.3h-2.4l-.5-2.3c-.6-.2-1.1-.4-1.6-.7l-2 1.2-1.7-1.7 1.2-2c-.3-.5-.5-1-.7-1.6L3.2 13v-2.4l2.3-.5c.2-.6.4-1.1.7-1.6L5 6.5l1.7-1.7 2 1.2c.5-.3 1-.5 1.6-.7L10.8 3Zm1.2 6a3 3 0 1 0 0 6 3 3 0 0 0 0-6Z" />
    </svg>
    Settings
  </summary>
  <div class="settings-popover">
    <fieldset>
      <legend>Theme</legend>
      <div class="option-grid">
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
    </fieldset>
    <fieldset>
      <legend>Refresh</legend>
      <div class="option-grid">
        {#each pollIntervals as interval}
          <button
            class:active={pollIntervalMs === interval}
            type="button"
            aria-pressed={pollIntervalMs === interval}
            onclick={() => onPollInterval(interval)}
          >
            {formatInterval(interval)}
          </button>
        {/each}
      </div>
    </fieldset>
    <fieldset>
      <legend>History</legend>
      <div class="option-grid">
        {#each historyPointOptions as option}
          <button
            class:active={historyPointLimit === option}
            type="button"
            aria-pressed={historyPointLimit === option}
            onclick={() => onHistoryLimit(option)}
          >
            {option}
          </button>
        {/each}
      </div>
    </fieldset>
    <label class="toggle-row">
      <input type="checkbox" checked={adminRequested} onchange={() => onAdminMode(!adminRequested)} />
      <span>Admin mode</span>
      <small>{adminEnabled ? "Active" : adminRequested ? "Requested" : "Off"}</small>
    </label>
    <button class="subtle-action" type="button" onclick={onResetHistory}>Reset history</button>
  </div>
</details>
