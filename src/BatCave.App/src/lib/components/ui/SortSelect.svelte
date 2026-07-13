<script lang="ts">
  import type { SortKey } from "../../process";

  export let options: { value: SortKey; label: string }[];
  export let value: SortKey;
  export let onChange: (value: SortKey) => void;
  export let disabled = false;
</script>

<select class="sort-select" aria-label="Process sort" {disabled} bind:value onchange={() => onChange(value)}>
  {#each options as option}
    <option value={option.value}>{option.label}</option>
  {/each}
</select>
