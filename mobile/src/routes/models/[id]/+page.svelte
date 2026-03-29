<script lang="ts">
  import { onMount } from 'svelte';
  import { meshOrganiserApi } from '$lib/api/meshOrganiserApi';
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import { derived } from 'svelte/store';
  import ModelViewer from '$lib/components/ModelViewer.svelte';
   
  let model = null;
  let loading = true;
  let error = null;
  const modelId = $page.params.id;
   
  async function loadModel() {
    try {
      loading = true;
      model = await meshOrganiserApi.getModel(Number(modelId));
      loading = false;
    } catch (err) {
      error = err;
      loading = false;
      console.error('Failed to load model:', err);
    }
  }
   
  onMount(loadModel);
   
  function sliceModel() {
    goto(`/models/${modelId}/slice`);
  }
   
  function printModel() {
    goto(`/models/${modelId}/print`);
  }
   
  function deleteModel() {
    // TODO: Implement delete functionality
    alert('Delete functionality not implemented yet');
  }
</script>

<div class="model-detail">
  {#if loading}
    <div class="loading">Loading model...</div>
  {:else if error}
    <div class="error">Error loading model: {error.message}</div>
    <button on:click={loadModel}>Retry</button>
  {:else if model}
    <div class="model-header">
      <h1 class="model-title">{model.name}</h1>
      <p class="model-meta">Added: {new Date(model.added).toLocaleDateString()}</p>
    </div>
    
<div class="model-content">
  {#if model.blob && model.blob.mimeType?.startsWith('image/')}
    <div class="model-image-container">
      <img 
        src={`data:${model.blob.mimeType};base64,${model.blob.data}`} 
        alt={model.name} 
        class="model-image"
      />
    </div>
  {:else}
    <div class="model-3d-container">
      <ModelViewer modelUri={`/api/models/${model.id}/blob`} />
    </div>
  {/if}
</div>
      {:else}
        <div class="model-3d-container">
          {/* 3D viewer would go here */}
          <div class="model-placeholder">
            3D Model Preview
          </div>
        </div>
      {/if}
      
      <div class="model-actions">
        <button class="action-button" on:click={sliceModel}>
          Slice Model
        </button>
        <button class="action-button" on:click={printModel}>
          Print Model
        </button>
        <button class="action-button danger" on:click={deleteModel}>
          Delete
        </button>
      </div>
    </div>
    
    <div class="model-info">
      <h2>Model Information</h2>
      <p><strong>Size:</strong> {(model.blob?.size / 1024 / 1024).toFixed(2)} MB</p>
      <p><strong>Format:</strong> {model.blob?.filetype?.toUpperCase() || 'Unknown'}</p>
      <p><strong>Description:</strong> {model.description || 'No description available'}</p>
      {#if model.group?.name}
        <p><strong>Group:</strong> {model.group.name}</p>
      {/if}
      {#if model.labels.length > 0}
        <p><strong>Labels:</strong> 
          {#each model.labels as label}
            <span class="label-tag" style="background-color: {label.color}; color: white; 
                     padding: 2px 6px; border-radius: 3px; font-size: 0.8em; margin: 0 2px;">
              {label.name}
            </span>
          {/each}
        </p>
      {/if}
    </div>
  {/if}
</div>

<style>
  .model-detail {
    padding: 1.5rem;
    max-width: 800px;
    margin: 0 auto;
  }
  
  .loading, .error {
    text-align: center;
    padding: 3rem;
    color: var(--text-muted);
  }
  
  .error {
    color: var(--error);
  }
  
  .model-header {
    text-align: center;
    margin-bottom: 2rem;
  }
  
  .model-title {
    font-size: 2rem;
    font-weight: bold;
    margin-bottom: 0.5rem;
  }
  
  .model-meta {
    color: var(--text-muted);
    font-size: 1rem;
  }
  
  .model-content {
    display: flex;
    flex-direction: column;
    gap: 1.5rem;
    margin-bottom: 2rem;
  }
  
  .model-image-container, .model-3d-container {
    width: 100%;
    height: 400px;
    background: var(--background-muted);
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 0.5rem;
    overflow: hidden;
  }
  
  .model-image {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
  }
  
  .model-placeholder {
    text-align: center;
    padding: 2rem;
    color: var(--text-muted);
    font-style: italic;
  }
  
  .model-actions {
    display: flex;
    gap: 1rem;
    flex-wrap: wrap;
  }
  
  .action-button {
    flex: 1;
    min-width: 120px;
    padding: 0.75rem 1.5rem;
    border: none;
    border-radius: 0.5rem;
    font-size: 1rem;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.2s;
  }
  
  .action-button:first-child {
    background: var(--primary);
    color: white;
  }
  
  .action-button:first-child:hover {
    background: var(--primary-dark);
  }
  
  .action-button:nth-child(2) {
    background: var(--secondary);
    color: white;
  }
  
  .action-button:nth-child(2):hover {
    background: var(--secondary-dark);
  }
  
  .action-button.danger {
    background: var(--error);
    color: white;
  }
  
  .action-button.danger:hover {
    background: var(--error-dark);
  }
  
  .model-info {
    background: var(--background-muted);
    padding: 1.5rem;
    border-radius: 0.5rem;
  }
  
  .model-info h2 {
    margin-top: 0;
    margin-bottom: 1rem;
    color: var(--text);
  }
  
  .model-info p {
    margin: 0.5rem 0;
    display: flex;
    justify-content: space-between;
  }
  
  .model-info p strong {
    width: 100px;
    display: inline-block;
  }
  
  .label-tag {
    display: inline-block;
    margin: 0 2px;
  }
</style>