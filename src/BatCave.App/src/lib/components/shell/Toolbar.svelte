<script lang="ts">
  import type { FocusMode, SortKey } from "../../process";
  import SegmentedControl from "../ui/SegmentedControl.svelte";
  import SortSelect from "../ui/SortSelect.svelte";
  import SettingsMenu from "./SettingsMenu.svelte";
  import type { ThemeOption, ThemePreference } from "../../themes";

  export let searchText: string;
  export let focusMode: FocusMode;
  export let sortKey: SortKey;
  export let isPaused: boolean;
  export let commandError: string;
  export let focusOptions: { value: FocusMode; label: string }[];
  export let sortOptions: { value: SortKey; label: string }[];
  export let themeOptions: ThemeOption[];
  export let themePreference: ThemePreference;
  export let pollIntervals: readonly number[];
  export let pollIntervalMs: number;
  export let historyPointOptions: readonly number[];
  export let historyPointLimit: number;
  export let adminRequested: boolean;
  export let adminEnabled: boolean;
  export let onSearch: (value: string) => void;
  export let onFocus: (mode: FocusMode) => void;
  export let onSort: (key: SortKey) => void;
  export let onPaused: () => void;
  export let onRefresh: () => void;
  export let onTheme: (theme: ThemePreference) => void;
  export let onPollInterval: (interval: number) => void;
  export let onHistoryLimit: (limit: number) => void;
  export let onAdminMode: (enabled: boolean) => void;
  export let onResetHistory: () => void;
</script>

<section class="toolbar" aria-label="Monitor controls">
  <label class="search-field" for="process-search">
    <span class="search-shell">
      <svg class="control-icon" viewBox="0 0 24 24" aria-hidden="true">
        <circle cx="11" cy="11" r="6" />
        <path d="m16 16 4 4" />
      </svg>
      <input
        id="process-search"
        class="search-input"
        value={searchText}
        oninput={(event) => onSearch(event.currentTarget.value)}
        aria-label="Search processes"
        placeholder="Search processes..."
        autocomplete="off"
      />
      <kbd>/</kbd>
    </span>
  </label>
  <div class="toolbar-group">
    <span>Focus</span>
    <SegmentedControl label="Process focus" options={focusOptions} value={focusMode} onChange={onFocus} />
  </div>
  <div class="toolbar-group">
    <span>Sort</span>
    <SortSelect options={sortOptions} value={sortKey} onChange={onSort} />
  </div>
  <div class="toolbar-actions">
    <button class="primary-action" class:resume={isPaused} type="button" onclick={onPaused}>
      {#if isPaused}
        <svg class="control-icon" viewBox="0 0 24 24" aria-hidden="true">
          <path d="M8 5v14l11-7z" />
        </svg>
        Resume
      {:else}
        <svg class="control-icon filled" viewBox="0 0 24 24" aria-hidden="true">
          <path d="M7 5h3v14H7zM14 5h3v14h-3z" />
        </svg>
        Pause
      {/if}
    </button>
    <button type="button" onclick={onRefresh}>
      <svg class="control-icon" viewBox="0 0 24 24" aria-hidden="true">
        <path d="M20 12a8 8 0 0 1-14.2 5" />
        <path d="M4 12A8 8 0 0 1 18.2 7" />
        <path d="M18 3v4h-4" />
        <path d="M6 21v-4h4" />
      </svg>
      Refresh
    </button>
    <SettingsMenu
      {themeOptions}
      {themePreference}
      {pollIntervals}
      {pollIntervalMs}
      {historyPointOptions}
      {historyPointLimit}
      {adminRequested}
      {adminEnabled}
      {onTheme}
      {onPollInterval}
      {onHistoryLimit}
      {onAdminMode}
      {onResetHistory}
    />
  </div>
  {#if commandError}
    <p class="command-error inline-command-error" role="alert">{commandError}</p>
  {/if}
</section>
