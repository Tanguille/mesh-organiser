<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { getContainer } from "$lib/api/dependency_injection";
  import { IModelApi, ModelStreamManager } from "$lib/api/shared/model_api";
  import Spinner from "$lib/components/view/spinner.svelte";

  let printJobs = $state<
    { id: number; name: string; status: string; progress: number }[]
  >([]);
  let loading = $state(true);
  let error = $state<string | null>(null);

  async function loadPrintJobs() {
    try {
      loading = true;
      error = null;
      const modelApi = getContainer().require<IModelApi>(IModelApi);
      // Get printed models - these act as "print jobs" in the mobile app
      const stream = new ModelStreamManager(modelApi, null, null, null, {
        printed: true,
        favorite: false,
      });
      const models = await stream.fetch();
      printJobs = models.map((m) => ({
        id: m.id,
        name: m.name,
        status: "completed",
        progress: 100,
      }));
      loading = false;
    } catch (err) {
      error = err instanceof Error ? err.message : String(err);
      loading = false;
      console.error("Failed to load print jobs:", err);
    }
  }

  function viewPrintJob(jobId: number) {
    goto(`/model?selected=${jobId}`);
  }

  onMount(loadPrintJobs);
</script>

<div class="space-y-6 px-4 pt-4">
  <div class="flex items-center justify-between">
    <h2 class="text-xl font-bold">Print Jobs</h2>
    <p class="text-sm text-muted-foreground">{printJobs.length} completed</p>
  </div>

  {#if loading}
    <div class="flex items-center justify-center py-12">
      <Spinner />
      <p class="ml-4">Loading print jobs...</p>
    </div>
  {:else if error}
    <div class="py-12 text-center">
      <p class="text-destructive">Error loading print jobs: {error}</p>
      <button onclick={loadPrintJobs} class="btn btn-primary mt-4">
        Retry
      </button>
    </div>
  {:else if printJobs.length === 0}
    <div class="py-12 text-center">
      <p class="text-muted-foreground">No completed prints yet.</p>
      <p class="mt-2 text-sm text-muted-foreground">
        Mark models as printed to track your print history.
      </p>
      <button onclick={() => goto("/slice")} class="btn btn-primary mt-4">
        Go to Slicer
      </button>
    </div>
  {:else}
    <div class="space-y-3">
      {#each printJobs as job}
        <button
          class="flex w-full items-center justify-between rounded-lg border border-border p-4 text-left transition-all hover:border-primary hover:bg-muted"
          onclick={() => viewPrintJob(job.id)}
        >
          <div class="min-w-0 flex-1 space-y-1">
            <h3 class="truncate font-medium">{job.name}</h3>
            <div class="flex items-center gap-2">
              <span
                class="inline-flex items-center rounded-full bg-green-100 px-2 py-0.5 text-xs font-medium text-green-800 dark:bg-green-900 dark:text-green-100"
              >
                {job.status}
              </span>
              <span class="text-xs text-muted-foreground">
                {job.progress}% complete
              </span>
            </div>
          </div>
          <svg
            class="ml-2 h-5 w-5 text-muted-foreground"
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
