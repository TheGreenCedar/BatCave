<script lang="ts">
  import {
    AppWindow,
    AppleLogo,
    ArrowsClockwise,
    Browser,
    ChatCircle,
    Code,
    Cube,
    Database,
    DesktopTower,
    FilmStrip,
    GraphicsCard,
    TerminalWindow,
    WindowsLogo,
  } from "phosphor-svelte";
  import type { ProcessIconKind } from "../../process";

  export let kind: ProcessIconKind = "process";
  export let child = false;
  export let src: string | undefined = undefined;

</script>

<span class:child class:has-image={Boolean(src) && kind !== "batcave"} class={`process-icon process-icon-${kind}`} aria-hidden="true">
  {#if kind === "batcave"}
    <svg viewBox="0 0 48 48">
      <path fill="currentColor" d="m2 18 9-7 7 3 4-5 2 6 2-6 4 5 7-3 9 7-7 2-5 11-6-4-4 5-4-5-6 4-5-11-7-2Z" />
      <path fill="currentColor" d="m11 11 7 3 4-5-1 12-10-1Z" opacity=".72" />
    </svg>
  {:else if src}
    <img src={src} alt="" />
  {:else if kind === "apple"}
    <AppleLogo size={22} weight="fill" />
  {:else if kind === "browser"}
    <Browser size={22} weight="regular" />
  {:else if kind === "code" || kind === "node"}
    <Code size={22} weight="bold" />
  {:else if kind === "container"}
    <Cube size={22} weight="regular" />
  {:else if kind === "database"}
    <Database size={22} weight="regular" />
  {:else if kind === "chat"}
    <ChatCircle size={22} weight="regular" />
  {:else if kind === "media"}
    <FilmStrip size={22} weight="regular" />
  {:else if kind === "sync"}
    <ArrowsClockwise size={22} weight="regular" />
  {:else if kind === "terminal"}
    <TerminalWindow size={22} weight="regular" />
  {:else if kind === "gpu"}
    <GraphicsCard size={22} weight="regular" />
  {:else if kind === "windows"}
    <WindowsLogo size={22} weight="fill" />
  {:else if child}
    <AppWindow size={21} weight="regular" />
  {:else}
    <DesktopTower size={22} weight="regular" />
  {/if}
</span>
