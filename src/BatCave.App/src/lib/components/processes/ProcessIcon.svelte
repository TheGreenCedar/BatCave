<script lang="ts">
  import AppWindow from "phosphor-svelte/lib/AppWindow";
  import AppleLogo from "phosphor-svelte/lib/AppleLogo";
  import ArrowsClockwise from "phosphor-svelte/lib/ArrowsClockwise";
  import Browser from "phosphor-svelte/lib/Browser";
  import ChatCircle from "phosphor-svelte/lib/ChatCircle";
  import Code from "phosphor-svelte/lib/Code";
  import Cube from "phosphor-svelte/lib/Cube";
  import Database from "phosphor-svelte/lib/Database";
  import DesktopTower from "phosphor-svelte/lib/DesktopTower";
  import FilmStrip from "phosphor-svelte/lib/FilmStrip";
  import GraphicsCard from "phosphor-svelte/lib/GraphicsCard";
  import TerminalWindow from "phosphor-svelte/lib/TerminalWindow";
  import WindowsLogo from "phosphor-svelte/lib/WindowsLogo";
  import brandIcon from "../../../../src-tauri/icons/64x64.png";
  import type { ProcessIconKind } from "../../process";

  export let kind: ProcessIconKind = "process";
  export let child = false;
  export let src: string | undefined = undefined;
  export let matched = false;
</script>

<span
  class:child
  class:has-image={Boolean(src) && kind !== "batcave"}
  class:matched
  class={`process-icon process-icon-${kind}`}
  title={matched ? "Icon matched from a related process" : undefined}
  aria-hidden="true"
>
  {#if kind === "batcave"}
    <img src={brandIcon} alt="" />
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
