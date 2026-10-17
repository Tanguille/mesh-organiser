<script lang="ts">
  import Button from "#lib/components/ui/button/button.svelte";
  import { IsMobile } from "#lib/hooks/is-mobile.svelte";
  import Undo2 from "@lucide/svelte/icons/undo-2";
  import type { Snippet } from "svelte";

  // Right-hand pane of the model/group grids; full width on mobile, where it
  // replaces the grid until closed.
  const props: { children: Snippet; onClose: () => void } = $props();
  const isMobile = new IsMobile();
</script>

<div
  class="{isMobile.current
    ? 'w-full'
    : 'w-[400px] min-w-[400px]'} relative mx-4 my-2 hide-scrollbar flex flex-col gap-4 overflow-y-auto"
>
  {#if isMobile.current}
    <Button onclick={props.onClose}>
      <Undo2 /> Close model preview
    </Button>
  {/if}
  {@render props.children()}
</div>
