<script lang="ts">
  import type { Snippet } from "svelte";

  const { children }: { children: Snippet } = $props();
</script>

<div
  class="flex h-full flex-col items-center justify-center rounded-md border border-dashed"
>
  <span class="text-xl">{@render children()}</span>
</div>
