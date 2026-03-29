<script lang="ts">
  import { onMount } from "svelte";
  import { onDestroy } from "svelte";

  let { modelUri = "" }: { modelUri?: string } = $props();

  let loading = $state(true);
  let error = $state<string | null>(null);
  let loadGeneration = $state(0);
  let lastSuccessfulGeometryKey = $state("");
  let pendingGeometryKey = $state("");

  // For mobile, we'll use a simpler approach - just display a placeholder
  async function loadModelData() {
    if (!modelUri) return;

    loading = true;
    error = null;

    try {
      // Simulate loading delay - in real app would fetch model
      await new Promise((resolve) => setTimeout(resolve, 1000));
      loading = false;
    } catch (err) {
      console.warn("Failed to load model:", err);
      error = err instanceof Error ? err.message : "Failed to load model";
      loading = false;
    }
  }

  function modelLoadKey(uri: string): string {
    return uri;
  }

  $effect(() => {
    if (!modelUri) return;

    const key = modelLoadKey(modelUri);
    if (key === lastSuccessfulGeometryKey) {
      return;
    }
    if (pendingGeometryKey === key) {
      return;
    }
    pendingGeometryKey = key;
    loadGeneration++;
    loadModelData();
  });

  onMount(() => {
    if (modelUri) {
      loadModelData();
    }
  });

  onDestroy(() => {
    // Cleanup if needed
  });
</script>

<div
  class="flex h-full w-full flex-col items-center justify-center rounded-lg bg-muted p-4 text-center"
>
  {#if loading}
    <div class="flex items-center justify-center py-8">
      <span class="text-xl">Loading model...</span>
    </div>
  {:else if error}
    <div class="flex items-center justify-center py-8">
      <span class="text-xl text-red-600">Failed to load model</span>
      <span class="mt-2 text-sm text-muted">{error}</span>
    </div>
  {:else}
    <div class="flex items-center justify-center py-8 text-center">
      <div
        class="mb-4 flex h-24 w-24 items-center justify-center rounded-lg bg-blue-100"
      >
        <svg
          class="h-6 w-6 text-blue-600"
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
      <p class="text-lg font-medium">3D Model Preview</p>
      <p class="text-sm text-muted">
        Interactive 3D viewer would be displayed here
      </p>
    </div>
  {/if}
</div>
