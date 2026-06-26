<script lang="ts" generics="T extends string | number">
  export let label: string;
  export let options: { value: T; label: string }[];
  export let value: T;
  export let onChange: (value: T) => void;
</script>

<div class="segmented" role="group" aria-label={label}>
  {#each options as option}
    <button
      class:active={value === option.value}
      type="button"
      aria-pressed={value === option.value}
      onclick={() => onChange(option.value)}
    >
      {option.label}
    </button>
  {/each}
</div>
