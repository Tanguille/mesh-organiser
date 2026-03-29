<script lang="ts">
  import { onMount } from 'svelte';
  import { meshOrganiserApi } from '$lib/api/meshOrganiserApi';
  import { Spinner } from '$lib/components/view/spinner.svelte';
  import { goto } from '$app/navigation';
  
  let printJobs = [];
  let loading = true;
  let error = null;
  
  async function loadPrintJobs() {
    try {
      loading = true;
      printJobs = await meshOrganiserApi.getPrintJobs();
      loading = false;
    } catch (err) {
      error = err;
      loading = false;
      console.error('Failed to load print jobs:', err);
    }
  }
  
  function viewPrintJob(jobId: string) {
    goto(`/print/${jobId}`);
  }
  
  function cancelPrintJob(jobId: string) {
    // TODO: Implement actual print job cancellation
    alert('Print job cancellation not implemented yet');
  }
  
  onMount(loadPrintJobs);
</script>

<div class="space-y-6">
  {#if loading}
    <div class="flex items-center justify-center py-12">
      <Spinner />
      <p class="ml-4">Loading print jobs...</p>
    </div>
  {:else if error}
    <div class="text-center py-12">
      <p class="text-error">Error loading print jobs: {error.message}</p>
      <button on:click={loadPrintJobs} class="mt-4 btn btn-primary">
        Retry
      </button>
    </div>
  {:else if printJobs.length === 0}
    <div class="text-center py-12">
      <p class="text-muted">No print jobs found.</p>
      <p class="text-sm text-muted">Print jobs will appear here when you start printing.</p>
    </div>
  {:else}
    <div class="flex justify-between items-center mb-4">
      <h2 class="text-xl font-bold">Print Jobs</h2>
      <p class="text-sm text-muted">{printJobs.length} print jobs</p>
    </div>
    
    <div class="space-y-4">
      {#each printJobs as job}
        <div class="border border-gray-200 rounded-lg p-4 hover:border-gray-300 transition-border cursor-pointer" 
             on:click={() => viewPrintJob(job.id)}>
          <div class="flex justify-between items-start mb-2">
            <div class="flex-1">
              <h3 class="font-medium">Print Job #{job.id.slice(0, 8)}...</h3>
              <p class="text-sm text-muted">
                Status: 
                <span class="px-2 py-0.5 rounded 
                  {#if job.status === 'pending'}bg-blue-100 text-blue-800
                  {:else if job.status === 'printing'}bg-green-100 text-green-800
                  {:else if job.status === 'paused'}bg-yellow-100 text-yellow-800
                  {:else if job.status === 'completed'}bg-green-100 text-green-800
                  {:else if job.status === 'failed'}bg-red-100 text-red-800
                  {/if}
                ">
                  {job.status}
                </span>
              </p>
            </div>
            <div class="flex-shrink-0">
              <button 
                on:click={(e) => { e.stopPropagation(); cancelPrintJob(job.id); }}
                class="btn btn-outline btn-sm"
              >
                Cancel
              </button>
            </div>
          </div>
          
          <div class="progress w-full bg-gray-200 rounded-full h-2.5">
            <div class="progress-bar bg-blue-600 h-2.5 rounded-full" style="width: {job.progress}%"></div>
          </div>
          
          <div class="mt-2 text-xs text-muted">
            Progress: {job.progress}%
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  /* Add any page-specific styles here */
</style>