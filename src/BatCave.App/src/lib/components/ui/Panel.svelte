<script lang="ts">
  export let className = "";
  export let ariaLabel: string | undefined = undefined;
</script>

<section class={`panel ${className}`} aria-label={ariaLabel}>
  <slot />
</section>
