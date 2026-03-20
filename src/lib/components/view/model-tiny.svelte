<script lang="ts">
  import { Card, CardHeader, CardContent } from "$lib/components/ui/card";

  import ModelImg from "$lib/components/view/model-img.svelte";
  import type { ClassValue } from "svelte/elements";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { flagsToGlyphObjects } from "$lib/glyph";
  import type { Model } from "$lib/api/shared/model_api";

  const props: { model: Model; class?: ClassValue } = $props();
</script>

<Card class="{props.class} relative">
  <CardHeader class="p-4">
    <h2 class="truncate text-center font-bold">{props.model.name}</h2>
  </CardHeader>
  <CardContent class="p-4">
    <ModelImg model={props.model} class="aspect-square w-full" />

    <div class="absolute bottom-2 left-2 flex flex-col gap-2">
      {#each flagsToGlyphObjects(props.model.flags) as glyph (glyph.id)}
        <Badge class={glyph.badgeClasses}
          ><glyph.glyph size="16" class={glyph.glyphClasses} /></Badge
        >
      {/each}
    </div>
  </CardContent>
</Card>
