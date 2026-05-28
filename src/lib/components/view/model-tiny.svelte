<script lang="ts">
  import { Card, CardHeader, CardContent } from "$lib/components/ui/card";

  import ModelImg from "$lib/components/view/model-img.svelte";
  import type { ClassValue } from "svelte/elements";
  import FlagBadges from "$lib/components/view/flag-badges.svelte";
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
      <FlagBadges flags={props.model.flags} />
    </div>
  </CardContent>
</Card>
