<script lang="ts">
  import ModelImg from "$lib/components/view/model-img.svelte";
  import type { ClassValue } from "svelte/elements";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { flagsToGlyphObjects } from "$lib/glyph";
  import type { Model } from "$lib/api/shared/model_api";
  import { configuration } from "$lib/configuration.svelte";

  const props: { model: Model; class?: ClassValue } = $props();
</script>

<div
  class="{props.class} flex min-w-0 flex-row gap-3 overflow-hidden rounded-lg border p-1 px-3"
>
  <ModelImg model={props.model} class="aspect-square h-full" />
  <div class="my-auto h-fit flex-1 overflow-hidden">
    <h2 class="truncate font-bold">{props.model.name}</h2>
    {#if configuration.show_date_on_list_view}
      <p class="hidden-if-small ml-4 text-xs font-thin">
        Added {props.model.added.toLocaleDateString()}
      </p>
    {/if}
  </div>

  <div class="my-auto flex h-fit flex-row gap-2">
    {#each flagsToGlyphObjects(props.model.flags) as glyph, idx (idx)}
      <Badge class={glyph.badgeClasses}
        ><glyph.glyph size="16" class={glyph.glyphClasses} /></Badge
      >
    {/each}
  </div>
</div>
