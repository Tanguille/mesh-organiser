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
  import Button from "$lib/components/ui/button/button.svelte";
  import { ChevronRight, File } from "lucide-svelte";

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
      <Button onclick={loadModels} class="mt-4">Retry</Button>
    </div>
  {:else if models.length === 0}
    <div class="py-12 text-center">
      <p class="text-muted-foreground">
        No models found. Import some models to get started.
      </p>
      <Button onclick={() => goto("/import")} class="mt-4">
        Import Models
      </Button>
    </div>
  {:else}
    <div class="space-y-3">
      {#each models as model (model.id)}
        <button
          class="flex w-full items-center space-x-4 rounded-lg border border-border p-3 text-left transition-all hover:border-primary hover:bg-muted"
          onclick={() => viewModel(model.id)}
        >
          <div
            class="flex h-14 w-14 shrink-0 items-center justify-center rounded bg-muted"
          >
            <File class="h-6 w-6 text-muted-foreground" />
          </div>
          <div class="min-w-0 flex-1 space-y-1">
            <h3 class="truncate font-medium">{model.name}</h3>
            <p class="text-sm text-muted-foreground">
              {formatSize(model.blob.size)} •
              {model.blob.filetype?.toUpperCase() || "Unknown"}
            </p>
          </div>
          <ChevronRight class="h-5 w-5 text-muted-foreground" />
        </button>
      {/each}
    </div>
  {/if}
</div>
