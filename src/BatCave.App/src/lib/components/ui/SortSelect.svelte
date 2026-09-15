<script lang="ts">
  import type { SortKey, SortOption } from "../../process";

  export let options: SortOption[];
  export let value: SortKey;
  export let onChange: (value: SortKey) => void;
  export let disabled = false;
</script>

<select
  class="sort-select"
  aria-label="Process sort"
  title={options.find((option) => option.value === value)?.description ?? "Sort workloads"}
  {disabled}
  bind:value
  onchange={() => onChange(value)}
>
  {#each options as option}
    <option value={option.value} title={option.description}>{option.label}</option>
  {/each}
</select>
