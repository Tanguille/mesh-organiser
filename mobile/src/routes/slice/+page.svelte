<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { meshOrganiserApi } from '$lib/api/meshOrganiserApi';
  import { Spinner } from '$lib/components/view/spinner.svelte';
  import { ModelLibrary } from '$lib/components/ModelLibrary.svelte';
  
  let models = [];
  let loading = true;
  let error = null;
  
  async function loadModels() {
    try {
      loading = true;
      models = await meshOrganiserApi.getModels();
      loading = false;
    } catch (err) {
      error = err;
      loading = false;
      console.error('Failed to load models:', err);
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
      <Spinner />
      <p class="ml-4">Loading models...</p>
    </div>
  {:else if error}
    <div class="text-center py-12">
      <p class="text-error">Error loading models: {error.message}</p>
      <button on:click={loadModels} class="mt-4 btn btn-primary">
        Retry
      </button>
    </div>
  {:else if models.length === 0}
    <div class="text-center py-12">
      <p class="text-muted">No models found. Import some models to get started.</p>
      <button on:click={() => goto('/import')} class="mt-4 btn btn-primary">
        Import Models
      </button>
    </div>
  {:else}
    <div class="flex justify-between items-center mb-4">
      <h2 class="text-xl font-bold">Select a Model to Slice</h2>
      <p class="text-sm text-muted">{models.length} models available</p>
    </div>
    
    <div class="space-y-4">
      {#each models as model}
        <div class="border border-gray-200 rounded-lg p-4 hover:border-gray-300 transition-border cursor-pointer" 
             on:click={() => viewModel(model.id)}>
          <div class="flex items-center space-x-4">
            {#if model.blob && model.blob.mimeType?.startsWith('image/')}
              <img 
                src={`data:${model.blob.mimeType};base64,${model.blob.data}`} 
                alt={model.name} 
                class="model-thumbnail"
                width="60"
                height="60"
                object-fit="cover"
                rounded
              />
            {:else}
              <div class="flex-shrink-0 h-12 w-12 bg-gray-200 rounded flex items-center justify-center">
                <svg class="h-6 w-6 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5a2 2 0 012-2V6a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z" />
                </svg>
              </div>
            {/if}
            <div class="flex-1 space-y-1">
              <h3 class="font-medium">{model.name}</h3>
              <p class="text-sm text-muted">
                {(model.blob?.size / 1024 / 1024).toFixed(2)} MB • 
                {model.blob?.filetype?.toUpperCase() || 'Unknown'}
              </p>
            </div>
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  /* Add any page-specific styles here */
</style>