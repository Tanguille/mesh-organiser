<script lang="ts">
  import ModelImg from "$lib/components/view/model-img.svelte";
  import type { ClassValue } from "svelte/elements";
  import FlagBadges from "$lib/components/view/flag-badges.svelte";
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
    <FlagBadges flags={props.model.flags} />
  </div>
</div>
