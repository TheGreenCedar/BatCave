<script lang="ts">
  import type { ThemeName } from "../../themes";

  export let themeName: ThemeName;
  export let accessibilityFixtureState: string | null = null;
</script>

<div
  class="app-shell"
  data-theme={themeName}
  data-accessibility-fixture={accessibilityFixtureState || undefined}
>
  <slot />
</div>
