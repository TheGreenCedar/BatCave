<script lang="ts">
  import type { ResolvedTheme } from "../../themes";

  export let theme: ResolvedTheme;
  export let accessibilityFixtureState: string | null = null;
</script>

<div
  class="app-shell"
  data-theme={theme.family}
  data-mode={theme.mode}
  data-accessibility-fixture={accessibilityFixtureState || undefined}
>
  <slot />
</div>
