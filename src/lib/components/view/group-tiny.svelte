<script lang="ts">
  import { Card, CardHeader, CardContent } from "$lib/components/ui/card";

  import GroupImg from "$lib/components/view/group-img.svelte";
  import type { ClassValue } from "svelte/elements";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { flagsToGlyphObjects } from "$lib/glyph";
  import type { Group } from "$lib/api/shared/group_api";

  const props: { group: Group; class?: ClassValue } = $props();
</script>

<Card class={props.class}>
  <CardHeader class="p-4">
    <h2
      class="w-100 overflow-hidden text-center font-bold text-ellipsis whitespace-nowrap"
    >
      {props.group.meta.name}
    </h2>
  </CardHeader>
  <CardContent class="relative p-4">
    <GroupImg model={props.group.models} class="aspect-square w-full" />
    {#if props.group.models.length >= 2}
      <Badge class="absolute right-2 bottom-2"
        >{props.group.models.length}</Badge
      >
    {/if}

    <div class="absolute bottom-2 left-2 flex flex-col gap-2">
      {#each flagsToGlyphObjects(props.group.flags) as glyph, idx (idx)}
        <Badge class={glyph.badgeClasses}
          ><glyph.glyph size="16" class={glyph.glyphClasses} /></Badge
        >
      {/each}
    </div>
  </CardContent>
</Card>
