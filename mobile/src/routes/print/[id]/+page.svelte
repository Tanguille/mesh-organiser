<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/stores';
  import { meshOrganiserApi } from '$lib/api/meshOrganiserApi';
  import { Spinner } from '$lib/components/view/spinner.svelte';
  import { toast } from 'svelte-sonner';
  
  let printJob = null;
  let loading = true;
  let error = null;
  const jobId = $page.params.id;
  
  let pollingInterval = null;
  
  async function loadPrintJob() {
    try {
      loading = true;
      printJob = await meshOrganiserApi.getPrintStatus(jobId);
      loading = false;
    } catch (err) {
      error = err;
      loading = false;
      console.error('Failed to load print job:', err);
      toast.error('Failed to load print job');
    }
  }
  
  function startPolling() {
    pollingInterval = setInterval(async () => {
      try {
        const updatedJob = await meshOrganiserApi.getPrintStatus(jobId);
        printJob = updatedJob;
        
        // Stop polling when job reaches terminal state
        if (['completed', 'failed', 'cancelled'].includes(updatedJob.status)) {
          clearInterval(pollingInterval);
          pollingInterval = null;
        }
      } catch (err) {
        console.error('Error polling print job:', err);
        clearInterval(pollingInterval);
        pollingInterval = null;
      }
    }, 5000); // Poll every 5 seconds
  }
  
  function stopPolling() {
    if (pollingInterval) {
      clearInterval(pollingInterval);
      pollingInterval = null;
    }
  }
  
  function cancelPrintJob() {
    // TODO: Implement actual print job cancellation via API
    alert('Print job cancellation not implemented yet');
  }
  
  function pausePrintJob() {
    // TODO: Implement print job pause via API
    alert('Print job pause not implemented yet');
  }
  
  function resumePrintJob() {
    // TODO: Implement print job resume via API
    alert('Print job resume not implemented yet');
  }
  
  onMount(() => {
    loadPrintJob();
    startPolling();
  });
  
  // Clean up interval on destroy
  // onDestroy(() => {
  //   stopPolling();
  // });
</script>

<div class="space-y-6">
  {#if loading}
    <div class="flex items-center justify-center py-12">
      <Spinner />
      <p class="ml-4">Loading print job...</p>
    </div>
  {:else if error}
    <div class="text-center py-12">
      <p class="text-error">Error loading print job: {error.message}</p>
      <button on:click={() => goto('/print')} class="mt-4 btn btn-primary">
        Back to Print Jobs
      </button>
    </div>
  {:else if printJob}
    <div>
      <h2 class="text-xl font-bold mb-4">Print Job Details</h2>
      
      <div class="border border-gray-200 rounded-lg p-6">
        <div class="space-y-4">
          <div class="flex justify-between items-start">
            <div>
              <h3 class="font-medium">Print Job #{printJob.id.slice(0, 8)}...</h3>
              <p class="text-sm text-muted">
                Status: 
                <span class="px-2 py-0.5 rounded 
                  {#if printJob.status === 'pending'}bg-blue-100 text-blue-800
                  {:else if printJob.status === 'printing'}bg-green-100 text-green-800
                  {:else if printJob.status === 'paused'}bg-yellow-100 text-yellow-800
                  {:else if printJob.status === 'completed'}bg-green-100 text-green-800
                  {:else if printJob.status === 'failed'}bg-red-100 text-red-800
                  {/if}
                ">
                  {printJob.status}
                </span>
              </p>
            </div>
          </div>
          
          <div class="progress w-full bg-gray-200 rounded-full h-2.5">
            <div class="progress-bar bg-blue-600 h-2.5 rounded-full" style="width: {printJob.progress}%"></div>
          </div>
          
          <div class="mt-2 text-xs text-muted">
            Progress: {printJob.progress}%
          </div>
          
          <div class="flex justify-between items-center mt-6">
            <button 
              on:click={pausePrintJob}
              class="btn btn-outline mr-2"
              disabled={printJob.status !== 'printing'}
            >
              Pause
            </button>
            <button 
              on:click={resumePrintJob}
              class="btn btn-outline mr-2"
              disabled={printJob.status !== 'paused'}
            >
              Resume
            </button>
            <button 
              on:click={cancelPrintJob}
              class="btn btn-error"
            >
              Cancel Print
            </button>
          </div>
          
          {#if printJob.status === 'failed' && printJob.error}
            <div class="mt-4 p-4 bg-red-50 rounded border border-red-200">
              <p class="text-red-600">Error: {printJob.error}</p>
            </div>
          {/if}
        </div>
      </div>
      
      <div class="mt-6">
        <button 
          on:click={() => goto('/print')}
          class="btn btn-outline w-full"
        >
          Back to Print Jobs
        </button>
      </div>
    </div>
  {/if}
</div>

<style>
  /* Add any page-specific styles here */
</style>