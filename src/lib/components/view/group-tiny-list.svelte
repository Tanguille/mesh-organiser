<script lang="ts">
  import ModelImg from "$lib/components/view/model-img.svelte";
  import type { ClassValue } from "svelte/elements";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { flagsToGlyphObjects } from "$lib/glyph";
  import type { Group } from "$lib/api/shared/group_api";
  import { configuration } from "$lib/configuration.svelte";

  const props: { group: Group; class?: ClassValue } = $props();
</script>

<div
  class="{props.class} flex min-w-0 flex-row gap-3 overflow-hidden rounded-lg border p-1 px-3"
>
  {#if configuration.only_show_single_image_in_groups}
    <ModelImg
      model={Array.from(props.group.models).sort(
        (a, b) => b.blob.size - a.blob.size,
      )[0]}
      class="aspect-square h-full"
    />
  {:else}
    <div class="imglist flex h-full flex-row gap-3">
      {#each props.group.models.slice(0, 3) as model (model.id)}
        <ModelImg {model} class="aspect-square h-full" />
      {/each}
    </div>
  {/if}
  <div class="my-auto h-fit flex-1 overflow-hidden">
    <h2 class="truncate font-bold">{props.group.meta.name}</h2>
    {#if configuration.show_date_on_list_view}
      <p class="hidden-if-small ml-4 text-xs font-thin">
        Created {props.group.meta.created.toLocaleDateString()}
      </p>
    {/if}
  </div>

  {#if props.group.models.length >= 2}
    <Badge class="my-auto h-fit">{props.group.models.length}</Badge>
  {/if}

  <div class="my-auto flex h-fit flex-row gap-2 empty:hidden">
    {#each flagsToGlyphObjects(props.group.flags) as glyph (glyph.id)}
      <Badge class={glyph.badgeClasses}
        ><glyph.glyph size="16" class={glyph.glyphClasses} /></Badge
      >
    {/each}
  </div>
</div>
