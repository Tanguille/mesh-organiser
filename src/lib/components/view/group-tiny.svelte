<script lang="ts">
  import { Card, CardHeader, CardContent } from "#lib/components/ui/card";

  import ModelImg from "#lib/components/view/model-img.svelte";
  import { configuration } from "#lib/configuration.svelte";
  import { representativeModel } from "#lib/utils";
  import type { ClassValue } from "svelte/elements";
  import { Badge } from "#lib/components/ui/badge/index.js";
  import FlagBadges from "#lib/components/view/flag-badges.svelte";
  import type { Group } from "#lib/api/shared/group_api";

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
    <div class="aspect-square w-full">
      {#if configuration.only_show_single_image_in_groups}
        <ModelImg
          model={representativeModel(props.group.models)}
          class="h-full w-full"
        />
      {:else if props.group.models.length <= 1}
        <ModelImg model={props.group.models[0]} class="h-full w-full" />
      {:else}
        <div
          class={props.group.models.length === 2
            ? "grid grid-cols-2 gap-1"
            : "grid grid-flow-col grid-cols-2 grid-rows-2 gap-1"}
        >
          {#each props.group.models.slice(0, 4) as model (model.id)}
            <ModelImg {model} class="h-full w-full" />
          {/each}
        </div>
      {/if}
    </div>
    {#if props.group.models.length >= 2}
      <Badge class="absolute right-2 bottom-2"
        >{props.group.models.length}</Badge
      >
    {/if}

    <div class="absolute bottom-2 left-2 flex flex-col gap-2">
      <FlagBadges flags={props.group.flags} />
    </div>
  </CardContent>
</Card>
