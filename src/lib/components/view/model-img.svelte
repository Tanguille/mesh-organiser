<script lang="ts">
  import { untrack } from "svelte";
  import type { ClassValue } from "svelte/elements";
  import Boxes from "@lucide/svelte/icons/boxes";
  import type { Model } from "$lib/api/shared/model_api";
  import { getContainer } from "$lib/api/dependency_injection";
  import { IBlobApi } from "$lib/api/shared/blob_api";
  import { configuration } from "$lib/configuration.svelte";

  let img_src = $state("");
  let load_failed = $state(false);
  let loadGeneration = 0;

  let props: { model: Model; class?: ClassValue } = $props();

  let blobApi = getContainer().require<IBlobApi>(IBlobApi);

  $effect(() => {
    const blob = $state.snapshot(props.model.blob);
    const gen = ++loadGeneration;
    untrack(async () => {
      try {
        img_src = await blobApi.getBlobThumbnailUrl(blob);
        if (gen !== loadGeneration) {
          return;
        }
        load_failed = false;
      } catch (e) {
        if (gen !== loadGeneration) {
          return;
        }
        load_failed = true;
        if (import.meta.env.DEV) {
          console.warn("Thumbnail load failed", e);
        }
      }
    });
  });
</script>

<div class={props.class}>
  {#if load_failed}
    <Boxes
      class="h-full w-full"
      style={`color: ${configuration.thumbnail_color};`}
    />
  {:else}
    <img
      src={img_src}
      onerror={() => (load_failed = true)}
      alt="Image of {props.model.name}"
    />
  {/if}
</div>
