<script lang="ts">
  export let tone: "accent" | "muted" | "warning" | "danger" = "muted";
</script>

<span class={`badge badge-${tone}`}>
  <slot />
</span>
