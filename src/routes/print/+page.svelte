<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { getContainer } from "$lib/api/dependency_injection";
  import {
    IModelApi,
    ModelStreamManager,
    type Model,
  } from "$lib/api/shared/model_api";
  import { toReadableSize } from "$lib/utils";
  import Spinner from "$lib/components/view/spinner.svelte";
  import Button from "$lib/components/ui/button/button.svelte";
  import ChevronRight from "@lucide/svelte/icons/chevron-right";
  import File from "@lucide/svelte/icons/file";

  let printedModels: Model[] = $state([]);
  let loading = $state(true);
  let error = $state<string | null>(null);

  async function loadPrintedModels() {
    try {
      loading = true;
      error = null;
      const modelApi = getContainer().require<IModelApi>(IModelApi);
      const stream = new ModelStreamManager(modelApi, null, null, null, {
        printed: true,
        favorite: false,
      });
      printedModels = await stream.fetch();
      loading = false;
    } catch (err) {
      error = err instanceof Error ? err.message : String(err);
      loading = false;
      console.error("Failed to load printed models:", err);
    }
  }

  function openModel(modelId: number) {
    goto(`/model?selected=${modelId}`);
  }

  onMount(loadPrintedModels);
</script>

<div class="space-y-6 px-4 pt-4 pb-24">
  <div class="flex items-center justify-between">
    <h2 class="text-xl font-bold">Printed models</h2>
    <p class="text-sm text-muted-foreground">
      {printedModels.length}
      {printedModels.length === 1 ? "model" : "models"}
    </p>
  </div>

  <p class="text-sm text-muted-foreground">
    Models you have marked as printed. If you slice on the server, G-code and
    slice outputs stay on the server; this list is your library view of finished
    work.
  </p>

  {#if loading}
    <div class="flex items-center justify-center py-12">
      <Spinner />
      <p class="ml-4">Loading…</p>
    </div>
  {:else if error}
    <div class="py-12 text-center">
      <p class="text-destructive">Error loading printed models: {error}</p>
      <Button onclick={loadPrintedModels} class="mt-4">Retry</Button>
    </div>
  {:else if printedModels.length === 0}
    <div class="py-12 text-center">
      <p class="text-muted-foreground">No printed models yet.</p>
      <p class="mt-2 text-sm text-muted-foreground">
        Mark a model as printed from its detail page to list it here.
      </p>
      <Button onclick={() => goto("/slice")} class="mt-4">Go to Slice</Button>
    </div>
  {:else}
    <div class="space-y-3">
      {#each printedModels as model (model.id)}
        <button
          class="flex w-full items-center space-x-4 rounded-lg border border-border p-3 text-left transition-all hover:border-primary hover:bg-muted"
          onclick={() => openModel(model.id)}
        >
          <div
            class="flex h-14 w-14 shrink-0 items-center justify-center rounded bg-muted"
          >
            <File class="h-6 w-6 text-muted-foreground" />
          </div>
          <div class="min-w-0 flex-1 space-y-1">
            <h3 class="truncate font-medium">{model.name}</h3>
            <p class="text-sm text-muted-foreground">
              {toReadableSize(model.blob.size)} •
              {model.blob.filetype?.toUpperCase() || "Unknown"}
            </p>
          </div>
          <ChevronRight class="h-5 w-5 text-muted-foreground" />
        </button>
      {/each}
    </div>
  {/if}
</div>
