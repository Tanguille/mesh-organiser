<script lang="ts">
  import type { Model } from "$lib/api/shared/model_api";
  import { configuration } from "$lib/configuration.svelte";
  import { representativeModel } from "$lib/utils";
  import ModelImg from "./model-img.svelte";
  import type { ClassValue } from "svelte/elements";

  let props: { model: Model[]; class?: ClassValue } = $props();
</script>

<div class={props.class}>
  {#if configuration.only_show_single_image_in_groups}
    <ModelImg model={representativeModel(props.model)} class="h-full w-full" />
  {:else if props.model.length <= 1}
    <ModelImg model={props.model[0]} class="h-full w-full" />
  {:else}
    <div
      class={props.model.length === 2
        ? "grid grid-cols-2 gap-1"
        : "grid grid-flow-col grid-cols-2 grid-rows-2 gap-1"}
    >
      {#each props.model.slice(0, 4) as model (model.id)}
        <ModelImg {model} class="h-full w-full" />
      {/each}
    </div>
  {/if}
</div>
