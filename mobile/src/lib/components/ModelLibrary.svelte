<script lang="ts">
  import { onMount } from "svelte";
  import { meshOrganiserApi } from "$lib/api/meshOrganiserApi";
  import ModelViewer from "./ModelViewer.svelte";
  import { goto } from "$app/navigation";
  import type { Model } from "$lib/shared/model_api";

  let models: Model[] = $state([]);
  let loading = $state(true);
  let error: Error | null = $state(null);

  async function loadModels() {
    try {
      loading = true;
      models = await meshOrganiserApi.getModels();
      loading = false;
    } catch (err) {
      error = err instanceof Error ? err : new Error(String(err));
      loading = false;
      console.error("Failed to load models:", err);
    }
  }

  onMount(loadModels);

  function viewModel(modelId: number) {
    goto(`/models/${modelId}`);
  }
</script>

<div class="mx-auto max-w-6xl p-4">
  {#if loading}
    <div class="p-8 text-center text-muted-foreground">Loading models...</div>
  {:else if error}
    <div class="p-8 text-center text-destructive">
      Error loading models: {error.message}
    </div>
    <button class="btn btn-primary" onclick={loadModels}>Retry</button>
  {:else if models.length === 0}
    <div class="p-8 text-center text-muted-foreground">
      No models found. Import some models to get started.
    </div>
  {:else}
    <div
      class="grid grid-cols-2 gap-4 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5"
    >
      {#each models as model (model.id)}
        <button
          type="button"
          class="cursor-pointer overflow-hidden rounded-lg border-0 bg-background p-0 text-left shadow-sm transition-all duration-200 hover:-translate-y-1 hover:shadow-md"
          onclick={() => viewModel(model.id)}
        >
          {#if model.blob && model.blob.mimeType?.startsWith("image/")}
            <img
              src={`data:${model.blob.mimeType};base64,${model.blob.data}`}
              alt={model.name}
              class="h-32 w-full rounded-t-lg object-cover"
            />
          {:else}
            <div
              class="flex h-32 w-full items-center justify-center rounded-t-lg bg-muted text-muted-foreground"
            >
              <ModelViewer modelUri={`/api/models/${model.id}/preview`} />
            </div>
          {/if}
          <div class="p-3">
            <h3 class="truncate text-sm font-semibold">{model.name}</h3>
            <p class="text-xs text-muted-foreground">
              {((model.blob?.size ?? 0) / 1024 / 1024).toFixed(2)} MB
            </p>
          </div>
        </button>
      {/each}
    </div>
  {/if}
</div>
