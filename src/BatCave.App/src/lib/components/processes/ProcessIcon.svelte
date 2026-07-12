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

  const brandIconUrl = new URL("../../../../src-tauri/icons/64x64.png", import.meta.url).href;
</script>

<span class:child class:has-image={Boolean(src) || kind === "batcave"} class={`process-icon process-icon-${kind}`} aria-hidden="true">
  {#if src}
    <img src={src} alt="" />
  {:else if kind === "batcave"}
    <img src={brandIconUrl} alt="" />
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
