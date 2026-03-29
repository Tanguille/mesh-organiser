<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { meshOrganiserApi } from "$lib/api/meshOrganiserApi";
  import type { Model } from "$lib/shared/model_api";

  let models: Model[] = [];
  let loading = true;
  let error: string | null = null;

  async function loadModels() {
    try {
      loading = true;
      models = await meshOrganiserApi.getModels();
      loading = false;
    } catch (err) {
      error = err instanceof Error ? err.message : String(err);
      loading = false;
      console.error("Failed to load models:", err);
    }
  }

  function viewModel(modelId: number) {
    goto(`/models/${modelId}/slice`);
  }

  onMount(loadModels);
</script>

<div class="space-y-6">
  {#if loading}
    <div class="flex items-center justify-center py-12">
      <div
        class="h-8 w-8 animate-spin rounded-full border-4 border-primary border-t-transparent"
      ></div>
      <p class="ml-4">Loading models...</p>
    </div>
  {:else if error}
    <div class="py-12 text-center">
      <p class="text-error">Error loading models: {error}</p>
      <button onclick={loadModels} class="btn btn-primary mt-4"> Retry </button>
    </div>
  {:else if models.length === 0}
    <div class="py-12 text-center">
      <p class="text-muted">
        No models found. Import some models to get started.
      </p>
      <button onclick={() => goto("/import")} class="btn btn-primary mt-4">
        Import Models
      </button>
    </div>
  {:else}
    <div class="mb-4 flex items-center justify-between">
      <h2 class="text-xl font-bold">Select a Model to Slice</h2>
      <p class="text-sm text-muted">{models.length} models available</p>
    </div>

    <div class="space-y-4">
      {#each models as model}
        <div
          class="transition-border cursor-pointer rounded-lg border border-gray-200 p-4 hover:border-gray-300"
          role="button"
          tabindex="0"
          onclick={() => viewModel(model.id)}
          onkeydown={(e) => e.key === "Enter" && viewModel(model.id)}
        >
          <div class="flex items-center space-x-4">
            {#if model.blob && model.blob.mimeType?.startsWith("image/")}
              <img
                src={`data:${model.blob.mimeType};base64,${model.blob.data}`}
                alt={model.name}
                class="model-thumbnail rounded"
                style="object-fit: cover; width: 60px; height: 60px;"
              />
            {:else}
              <div
                class="flex h-12 w-12 flex-shrink-0 items-center justify-center rounded bg-gray-200"
              >
                <svg
                  class="h-6 w-6 text-gray-400"
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
            {/if}
            <div class="flex-1 space-y-1">
              <h3 class="font-medium">{model.name}</h3>
              <p class="text-sm text-muted">
                {(model.blob?.size / 1024 / 1024).toFixed(2)} MB •
                {model.blob?.filetype?.toUpperCase() || "Unknown"}
              </p>
            </div>
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>
