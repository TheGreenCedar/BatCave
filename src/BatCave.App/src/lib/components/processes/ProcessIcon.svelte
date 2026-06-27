<script lang="ts">
  import type { ProcessIconKind } from "../../process";

  export let kind: ProcessIconKind = "process";
  export let child = false;
  export let src: string | undefined = undefined;
</script>

<span class:child class={`process-icon process-icon-${kind}`} aria-hidden="true">
  {#if src}
    <img src={src} alt="" />
  {:else if kind === "batcave"}
    <svg viewBox="0 0 24 24">
      <path d="M3 9.2 7.2 6l2.7 2.1L12 5.3l2.1 2.8L16.8 6 21 9.2l-3.4 4-3.1-1.3L12 16.7l-2.5-4.8-3.1 1.3Z" />
    </svg>
  {:else if kind === "browser"}
    <svg viewBox="0 0 24 24">
      <circle cx="12" cy="12" r="7.5" />
      <path d="M5 9h14M9.2 19.1 13 12 9 5.1M14.8 19.1 11 12l4-6.9" />
    </svg>
  {:else if kind === "code"}
    <svg viewBox="0 0 24 24">
      <path d="m9.2 8.2-4 3.8 4 3.8M14.8 8.2l4 3.8-4 3.8M13.2 6.8l-2.4 10.4" />
    </svg>
  {:else if kind === "container"}
    <svg viewBox="0 0 24 24">
      <path d="M5 13.5h13.5l1.5-3h-3l-1-2H5Z" />
      <path d="M7 8.5V6h2v2.5M10.5 8.5V6h2v2.5M14 8.5V6h2v2.5M7.3 17h8.4" />
    </svg>
  {:else if kind === "database"}
    <svg viewBox="0 0 24 24">
      <ellipse cx="12" cy="6.5" rx="6.5" ry="2.8" />
      <path d="M5.5 6.5v8.8c0 1.6 2.9 2.8 6.5 2.8s6.5-1.2 6.5-2.8V6.5M5.5 11c0 1.6 2.9 2.8 6.5 2.8s6.5-1.2 6.5-2.8" />
    </svg>
  {:else if kind === "chat"}
    <svg viewBox="0 0 24 24">
      <path d="M5.5 6.5h13v8.8h-7.1L7 18.5v-3.2H5.5Z" />
      <path d="M8.5 10.2h7M8.5 12.8h4.8" />
    </svg>
  {:else if kind === "media"}
    <svg viewBox="0 0 24 24">
      <circle cx="12" cy="12" r="7.5" />
      <path d="M9.8 8.5v7l6-3.5Z" />
    </svg>
  {:else if kind === "node"}
    <svg viewBox="0 0 24 24">
      <path d="M12 4.8 18.3 8v8L12 19.2 5.7 16V8Z" />
      <path d="M9.1 14.7V9.3l5.8 5.4V9.3" />
    </svg>
  {:else if kind === "sync"}
    <svg viewBox="0 0 24 24">
      <path d="M7.8 8.3A5.6 5.6 0 0 1 17 9.7l1.3 1.4M16.2 15.7A5.6 5.6 0 0 1 7 14.3l-1.3-1.4" />
      <path d="M18.4 7.3v3.8h-3.8M5.6 16.7v-3.8h3.8" />
    </svg>
  {:else if kind === "gpu"}
    <svg viewBox="0 0 24 24">
      <rect x="5" y="7" width="11.5" height="10" rx="1.5" />
      <path d="M8 10h5.5v4H8ZM16.5 10.5H20M16.5 14H20M8 4.8v2.2M12 4.8v2.2M8 17v2.2M12 17v2.2" />
    </svg>
  {:else if kind === "windows"}
    <svg viewBox="0 0 24 24">
      <path d="m5 6.8 5.8-.8v5.5H5Zm7.2-1 6.8-1v6.7h-6.8ZM5 12.7h5.8v5.5L5 17.4Zm7.2 0H19v6.5l-6.8-.9Z" />
    </svg>
  {:else}
    <svg viewBox="0 0 24 24">
      <rect x="5" y="5.5" width="14" height="13" rx="2" />
      <path d="M8 12h3l2-3 3 6h2" />
    </svg>
  {/if}
</span>
