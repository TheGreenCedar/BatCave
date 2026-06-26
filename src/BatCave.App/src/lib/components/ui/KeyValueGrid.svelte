<script lang="ts">
  export let items: { label: string; value: string; title?: string }[] = [];
</script>

<dl class="key-value-grid">
  {#each items as item}
    <div>
      <dt>{item.label}</dt>
      <dd title={item.title}>{item.value}</dd>
    </div>
  {/each}
</dl>
