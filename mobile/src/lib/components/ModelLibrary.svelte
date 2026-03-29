<script lang="ts">
  import { onMount } from 'svelte';
  import { meshOrganiserApi } from '$lib/api/meshOrganiserApi';
  import ModelViewer from './ModelViewer.svelte';
  import { goto } from '$app/navigation';
  
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
  
  onMount(loadModels);
  
  function viewModel(modelId) {
    goto(`/models/${modelId}`);
  }
</script>

<div class="model-library">
  {#if loading}
    <div class="loading">Loading models...</div>
  {:else if error}
    <div class="error">Error loading models: {error.message}</div>
    <button on:click={loadModels}>Retry</button>
  {:else if models.length === 0}
    <div class="empty">No models found. Import some models to get started.</div>
  {:else}
    <div class="models-grid">
      {#each models as model}
        <div class="model-card" on:click={() => viewModel(model.id)}>
          {#if model.blob && model.blob.mimeType?.startsWith('image/')}
            <img 
              src={`data:${model.blob.mimeType};base64,${model.blob.data}`} 
              alt={model.name} 
              class="model-thumbnail"
            />
          {:else}
            <div class="model-placeholder">
              <ModelViewer modelUri={`/api/models/${model.id}/preview`} />
            </div>
          {/if}
          <div class="model-info">
            <h3 class="model-name">{model.name}</h3>
            <p class="model-size">
              {(model.blob?.size / 1024 / 1024).toFixed(2)} MB
            </p>
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .model-library {
    padding: 1rem;
    max-width: 1200px;
    margin: 0 auto;
  }
  
  .loading, .error, .empty {
    text-align: center;
    padding: 2rem;
    color: var(--text-muted);
  }
  
  .error {
    color: var(--error);
  }
  
  .models-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: 1.5rem;
  }
  
  .model-card {
    background: var(--background);
    border-radius: 0.5rem;
    overflow: hidden;
    box-shadow: 0 2px 4px rgba(0,0,0,0.1);
    transition: transform 0.2s, box-shadow 0.2s;
    cursor: pointer;
  }
  
  .model-card:hover {
    transform: translateY(-4px);
    box-shadow: 0 4px 8px rgba(0,0,0,0.15);
  }
  
  .model-thumbnail {
    width: 100%;
    height: 150px;
    object-fit: cover;
  }
  
  .model-placeholder {
    width: 100%;
    height: 150px;
    background: var(--background-muted);
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-muted);
  }
  
  .model-info {
    padding: 1rem;
  }
  
  .model-name {
    margin: 0 0 0.5rem 0;
    font-size: 1rem;
    font-weight: 600;
  }
  
  .model-size {
    margin: 0;
    font-size: 0.875rem;
    color: var(--text-muted);
  }
</style>