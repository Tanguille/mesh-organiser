<script lang="ts">
  import { onMount } from "svelte";
  import type { Snippet } from "svelte";
  import type { ClassValue } from "svelte/elements";

  let containerRef: HTMLDivElement | null = $state(null);
  let contentRef: HTMLSpanElement | null = $state(null);
  let overflow = $state(false);

  const props: { children: Snippet; class?: ClassValue } = $props();

  onMount(() => {
    if (containerRef && contentRef) {
      overflow = contentRef.scrollWidth > containerRef.clientWidth;
    }
  });
</script>

<div bind:this={containerRef} class="{props.class} overflow-hidden">
  <div
    bind:this={contentRef}
    class="w-100 overflow-hidden font-bold whitespace-nowrap {overflow
      ? 'inline-block animate-marquee'
      : ''}"
  >
    <span>
      {@render props.children?.()}
    </span>
    <span>
      {@render props.children?.()}
    </span>
  </div>
</div>
