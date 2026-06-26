<script lang="ts">
  import type { ThemeName } from "../../themes";

  export let themeName: ThemeName;
</script>

<main class="app-shell" data-theme={themeName}>
  <slot />
</main>
