<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { page } from "$app/stores";
  import { meshOrganiserApi } from "$lib/api/meshOrganiserApi";
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  import type { Printer } from "$lib/api/meshOrganiserApi";
  import { toast } from "svelte-sonner";

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let model: any = $state(null);
  let loading = $state(true);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let error: any = $state(null);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const modelId: any = $page.params.id;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const params: any = $page.params;
  const slicedUrl = decodeURIComponent(params.url ?? "");

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let printers: any[] = $state([]);
  let loadingPrinters = $state(true);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let errorPrinters: any = $state(null);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let selectedPrinterId: any = $state(null);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let printJobId: any = $state(null);
  let printStatus: string | null = $state(null);
  let printProgress = $state(0);
  let printError: string | null = $state(null);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let pollingInterval: any = $state(null);

  // Helper functions for dynamic classes
  function getStatusClass(status: string) {
    if (status === "idle") return "bg-green-100 text-green-800";
    if (status === "printing") return "bg-blue-100 text-blue-800";
    if (status === "paused") return "bg-yellow-100 text-yellow-800";
    return "bg-red-100 text-red-800";
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  function getButtonClass(printerId: any) {
    return selectedPrinterId === printerId
      ? "btn btn-primary"
      : "btn btn-outline";
  }

  async function loadModel() {
    try {
      loading = true;
      model = await meshOrganiserApi.getModel(modelId);
      loading = false;
    } catch (err) {
      error = err instanceof Error ? err.message : String(err);
      loading = false;
      console.error("Failed to load model:", err);
      toast.error("Failed to load model");
    }
  }

  async function loadPrinters() {
    try {
      loadingPrinters = true;
      printers = await meshOrganiserApi.getPrinters();
      loadingPrinters = false;
    } catch (err) {
      errorPrinters = err instanceof Error ? err.message : String(err);
      loadingPrinters = false;
      console.error("Failed to load printers:", err);
      toast.error("Failed to load printers");
    }
  }

  async function startPrint() {
    if (!selectedPrinterId) return;

    try {
      const result = await meshOrganiserApi.startPrint(
        selectedPrinterId,
        modelId,
      );
      printJobId = result.id;

      // Start polling for print status
      pollingInterval = setInterval(async () => {
        try {
          const status = await meshOrganiserApi.getPrintStatus(printJobId!);
          printStatus = status.status;
          printProgress = status.progress;

          if (["completed", "failed", "cancelled"].includes(status.status)) {
            clearInterval(pollingInterval);
            pollingInterval = null;

            if (status.status === "failed") {
              // eslint-disable-next-line @typescript-eslint/no-explicit-any
              printError = (status as any).error || "Print failed";
              toast.error("Print failed");
            } else if (status.status === "completed") {
              toast.success("Print completed successfully");
            }
          }
        } catch (err) {
          console.error("Error getting print status:", err);
          clearInterval(pollingInterval);
          pollingInterval = null;
          printError = "Failed to get print status";
          toast.error("Failed to get print status");
        }
      }, 5000); // Poll every 5 seconds
    } catch (err) {
      printError = err instanceof Error ? err.message : "Failed to start print";
      console.error("Print start error:", err);
      toast.error("Failed to start print");
    }
  }

  function cancelPrint() {
    if (printJobId && pollingInterval) {
      clearInterval(pollingInterval);
      pollingInterval = null;
      // TODO: Implement actual print cancellation via API
      toast.info("Print cancelled");
      goto(`/`);
    }
  }

  function pausePrint() {
    // TODO: Implement print pause via API
    toast.info("Print paused");
  }

  function resumePrint() {
    // TODO: Implement print resume via API
    toast.info("Print resumed");
  }

  onMount(async () => {
    await loadModel();
    await loadPrinters();
  });

  // Clean up interval on destroy
  // onDestroy(() => {
  //   if (pollingInterval) {
  //     clearInterval(pollingInterval);
  //   }
  // });
</script>

<div class="space-y-6">
  {#if loading}
    <div class="flex items-center justify-center py-12">
      <div
        class="h-8 w-8 animate-spin rounded-full border-4 border-primary border-t-transparent"
      ></div>
      <p class="ml-4">Loading model...</p>
    </div>
  {:else if error}
    <div class="py-12 text-center">
      <p class="text-error">Error loading model: {error}</p>
      <button onclick={loadModel} class="btn btn-primary mt-4"> Retry </button>
    </div>
  {:else if model}
    <div>
      <h2 class="mb-4 text-xl font-bold">{model.name}</h2>

      <div class="space-y-4">
        <h3 class="mb-2 text-lg font-medium">Select Printer</h3>

        {#if loadingPrinters}
          <div class="flex items-center justify-center py-8">
            <div
              class="h-8 w-8 animate-spin rounded-full border-4 border-primary border-t-transparent"
            ></div>
            <p class="ml-4">Loading printers...</p>
          </div>
        {:else if errorPrinters}
          <div class="py-8 text-center">
            <p class="text-error">
              Error loading printers: {errorPrinters}
            </p>
          </div>
        {:else if printers.length === 0}
          <div class="py-8 text-center">
            <p class="text-warning">No printers configured</p>
            <p class="text-sm text-muted">
              Please configure printers in the web interface
            </p>
          </div>
        {:else}
          <div class="space-y-4">
            {#each printers as printer}
              <div class="rounded-lg border border-gray-200 p-4">
                <div class="flex items-start justify-between">
                  <div>
                    <h4 class="font-medium">{printer.name}</h4>
                    <p class="text-sm text-muted">
                      Status:
                      <span
                        class="rounded px-2 py-0.5 {getStatusClass(
                          printer.status,
                        )}"
                      >
                        {printer.status}
                      </span>
                    </p>
                  </div>
                  <div class="flex-shrink-0">
                    <button
                      onclick={() => (selectedPrinterId = printer.id)}
                      class={getButtonClass(printer.id)}
                    >
                      {selectedPrinterId === printer.id ? "Selected" : "Select"}
                    </button>
                  </div>
                </div>
              </div>
            {/each}
          </div>
        {/if}

        <div class="mt-6">
          <button
            onclick={startPrint}
            class="btn btn-primary w-full"
            disabled={!selectedPrinterId || loadingPrinters}
          >
            {#if loadingPrinters}
              Starting...
            {:else if !selectedPrinterId}
              Select a printer to start
            {:else}
              Start Print
            {/if}
          </button>
        </div>
      </div>

      {#if printJobId}
        <div class="rounded-lg border border-gray-200 p-4">
          <h3 class="mb-4 text-lg font-medium">Print Job Status</h3>

          <div class="space-y-4">
            <div class="flex items-center">
              <div class="h-12 w-12">
                {#if printStatus === "printing"}
                  <div
                    class="h-8 w-8 animate-spin rounded-full border-4 border-primary border-t-transparent"
                  ></div>
                {:else if printStatus === "completed"}
                  <svg
                    class="h-6 w-6 text-green-600"
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24"
                  >
                    <path
                      stroke-linecap="round"
                      stroke-linejoin="round"
                      stroke-width="2"
                      d="M5 13l4 4L19 7"
                    />
                  </svg>
                {:else if printStatus === "failed"}
                  <svg
                    class="h-6 w-6 text-red-600"
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24"
                  >
                    <path
                      stroke-linecap="round"
                      stroke-linejoin="round"
                      stroke-width="2"
                      d="M6 18L18 6M6 6l12 12"
                    />
                  </svg>
                {:else}
                  <svg
                    class="h-6 w-6 text-gray-600"
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24"
                  >
                    <path
                      stroke-linecap="round"
                      stroke-linejoin="round"
                      stroke-width="2"
                      d="M12 8v4l4 4"
                    />
                  </svg>
                {/if}
              </div>
              <div class="ml-4">
                <p class="font-medium">Status: {printStatus}</p>
                <p class="text-sm text-muted">Progress: {printProgress}%</p>
              </div>
            </div>

            <div class="progress h-2.5 w-full rounded-full bg-gray-200">
              <div
                class="progress-bar h-2.5 rounded-full bg-blue-600"
                style="width: {printProgress}%"
              ></div>
            </div>

            <div class="mt-4 flex items-center justify-between">
              <button
                onclick={pausePrint}
                class="btn btn-outline mr-2"
                disabled={printStatus !== "printing"}
              >
                Pause
              </button>
              <button
                onclick={resumePrint}
                class="btn btn-outline mr-2"
                disabled={printStatus !== "paused"}
              >
                Resume
              </button>
              <button onclick={cancelPrint} class="btn btn-error">
                Cancel Print
              </button>
            </div>

            {#if printError}
              <div class="mt-4 rounded border border-red-200 bg-red-50 p-4">
                <p class="text-red-600">Error: {printError}</p>
              </div>
            {/if}
          </div>
        </div>
      {/if}
    </div>
  {/if}
</div>
