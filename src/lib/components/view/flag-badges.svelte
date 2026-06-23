<script lang="ts">
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { flagsToGlyphObjects } from "$lib/glyph";
  import type { ModelFlags } from "$lib/api/shared/model_api";

  const props: { flags: ModelFlags } = $props();
</script>

{#each flagsToGlyphObjects(props.flags) as glyph (glyph.id)}
  <Badge class={glyph.badgeClasses}
    ><glyph.glyph size="16" class={glyph.glyphClasses} /></Badge
  >
{/each}
