<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { getContainer } from "$lib/api/dependency_injection";
  import {
    IModelApi,
    ModelOrderBy,
    type Model,
  } from "$lib/api/shared/model_api";
  import Spinner from "$lib/components/view/spinner.svelte";

  let models: Model[] = $state([]);
  let loading = $state(true);
  let error = $state<string | null>(null);

  async function loadModels() {
    try {
      loading = true;
      error = null;
      const modelApi = getContainer().require<IModelApi>(IModelApi);
      models = await modelApi.getModels(
        null,
        null,
        null,
        ModelOrderBy.AddedDesc,
        null,
        1,
        100,
        null,
      );
      loading = false;
    } catch (err) {
      error = err instanceof Error ? err.message : String(err);
      loading = false;
      console.error("Failed to load models:", err);
    }
  }

  function viewModel(modelId: number) {
    goto(`/model?selected=${modelId}`);
  }

  function formatSize(bytes: number): string {
    if (bytes < 1024) return bytes + " B";
    if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + " KB";
    return (bytes / 1024 / 1024).toFixed(2) + " MB";
  }

  onMount(loadModels);
</script>

<div class="space-y-6 px-4 pt-4">
  <div class="flex items-center justify-between">
    <h2 class="text-xl font-bold">Select a Model to Slice</h2>
    <p class="text-sm text-muted-foreground">
      {models.length} models available
    </p>
  </div>

  {#if loading}
    <div class="flex items-center justify-center py-12">
      <Spinner />
      <p class="ml-4">Loading models...</p>
    </div>
  {:else if error}
    <div class="py-12 text-center">
      <p class="text-destructive">Error loading models: {error}</p>
      <button onclick={loadModels} class="btn btn-primary mt-4"> Retry </button>
    </div>
  {:else if models.length === 0}
    <div class="py-12 text-center">
      <p class="text-muted-foreground">
        No models found. Import some models to get started.
      </p>
      <button onclick={() => goto("/import")} class="btn btn-primary mt-4">
        Import Models
      </button>
    </div>
  {:else}
    <div class="space-y-3">
      {#each models as model}
        <button
          class="flex w-full items-center space-x-4 rounded-lg border border-border p-3 text-left transition-all hover:border-primary hover:bg-muted"
          onclick={() => viewModel(model.id)}
        >
          <div
            class="flex h-14 w-14 flex-shrink-0 items-center justify-center rounded bg-muted"
          >
            <svg
              class="h-6 w-6 text-muted-foreground"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                stroke-linecap="round"
                stroke-linejoin="round"
                stroke-width="2"
                d="M9 12h6m-6 4h6m2 5a2 2 0 012-2V6a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"
              />
            </svg>
          </div>
          <div class="min-w-0 flex-1 space-y-1">
            <h3 class="truncate font-medium">{model.name}</h3>
            <p class="text-sm text-muted-foreground">
              {formatSize(model.blob.size)} •
              {model.blob.filetype?.toUpperCase() || "Unknown"}
            </p>
          </div>
          <svg
            class="h-5 w-5 text-muted-foreground"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
          >
            <path
              stroke-linecap="round"
              stroke-linejoin="round"
              stroke-width="2"
              d="M9 5l7 7-7 7"
            />
          </svg>
        </button>
      {/each}
    </div>
  {/if}
</div>
