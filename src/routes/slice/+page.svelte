<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { getContainer } from "#lib/api/dependency_injection";
  import { FileType } from "#lib/api/shared/blob_api";
  import {
    defaultModelFilter,
    IModelApi,
    MAX_PAGE_SIZE,
    modelStream,
    ModelOrderBy,
    type Model,
  } from "#lib/api/shared/model_api";
  import { ISlicerApi } from "#lib/api/shared/slicer_api";
  import { toReadableSize } from "#lib/utils";
  import Spinner from "#lib/components/view/spinner.svelte";
  import Button from "#lib/components/ui/button/button.svelte";
  import { Label } from "#lib/components/ui/label/index.js";
  import ChevronRight from "@lucide/svelte/icons/chevron-right";
  import File from "@lucide/svelte/icons/file";
  import LoaderCircle from "@lucide/svelte/icons/loader-circle";
  import { toast } from "svelte-sonner";

  let models: Model[] = $state([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  // Only the mobile remote client registers a slicer that can slice on the server.
  const slicerApi = getContainer().optional<ISlicerApi>(ISlicerApi);
  const canSliceOnServer = typeof slicerApi?.sliceOnServer === "function";

  let selectedModelId = $state<number | null>(null);
  let slicing = $state(false);

  async function loadModels() {
    try {
      loading = true;
      error = null;
      const modelApi = getContainer().require<IModelApi>(IModelApi);
      const all: Model[] = [];
      for await (const page of modelStream(
        modelApi,
        defaultModelFilter({
          orderBy: ModelOrderBy.AddedDesc,
          // G-code is already sliced output, so keep it out of the picker.
          fileTypes: [
            FileType.STL,
            FileType.OBJ,
            FileType.THREEMF,
            FileType.STEP,
          ],
        }),
        MAX_PAGE_SIZE,
      )) {
        all.push(...page);
      }
      models = all;
      loading = false;
    } catch (err) {
      error = err instanceof Error ? err.message : String(err);
      loading = false;
      console.error("Failed to load models:", err);
    }
  }

  async function openInSlicer(model: Model) {
    if (!slicerApi) {
      toast.error("No slicer is available in this build.");

      return;
    }

    await slicerApi.openInSlicer([model]);
  }

  async function onSliceOnServer() {
    if (selectedModelId === null) {
      toast.error("Select a model to slice.");

      return;
    }

    if (!slicerApi?.sliceOnServer) {
      toast.error("Server slicing is not available.");

      return;
    }

    slicing = true;
    try {
      const res = await slicerApi.sliceOnServer(selectedModelId);
      if (res.success) {
        toast.success("Slice finished on server.", {
          description:
            res.message ??
            `Output model id ${res.outputModelId}. G-code stays on the server.`,
        });
      } else {
        toast.error(res.message ?? "Slice failed.");
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      toast.error("Could not slice on server.", { description: msg });
      console.error(err);
    } finally {
      slicing = false;
    }
  }

  onMount(loadModels);
</script>

<div class="space-y-6 px-4 pt-4 pb-24">
  <div class="flex items-center justify-between">
    <h2 class="text-xl font-bold">
      {canSliceOnServer ? "Slice on server" : "Select a Model to Slice"}
    </h2>
    <p class="text-sm text-muted-foreground">
      {models.length} sliceable models
    </p>
  </div>

  {#if canSliceOnServer}
    <p class="text-sm text-muted-foreground">
      G-code is generated on your Mesh Organiser server. Pick a model, then
      slice.
    </p>
  {:else}
    <p class="text-sm text-muted-foreground">
      Tap a model to open it in your slicer.
    </p>
  {/if}

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
  {:else if canSliceOnServer}
    <div class="space-y-4">
      <div class="space-y-2">
        <Label id="slice-model-label">Model</Label>
        <div
          role="group"
          aria-labelledby="slice-model-label"
          class="max-h-48 space-y-2 overflow-y-auto rounded-lg border border-border p-2"
        >
          {#each models as model (model.id)}
            <button
              type="button"
              aria-pressed={selectedModelId === model.id}
              class="flex w-full items-center space-x-3 rounded-md p-2 text-left transition-colors hover:bg-muted {selectedModelId ===
              model.id
                ? 'bg-muted ring-2 ring-primary ring-offset-2 ring-offset-background'
                : ''}"
              onclick={() => {
                selectedModelId = model.id;
              }}
            >
              <div
                class="flex h-10 w-10 shrink-0 items-center justify-center rounded bg-muted"
              >
                <File class="h-5 w-5 text-muted-foreground" />
              </div>
              <div class="min-w-0 flex-1">
                <p class="truncate font-medium">{model.name}</p>
                <p class="text-xs text-muted-foreground">
                  {toReadableSize(model.blob.size)}
                </p>
              </div>
            </button>
          {/each}
        </div>
      </div>

      <Button
        class="w-full"
        disabled={selectedModelId === null || slicing}
        onclick={onSliceOnServer}
      >
        {#if slicing}
          <span class="inline-flex items-center gap-2">
            <LoaderCircle class="h-4 w-4 shrink-0 animate-spin" />
            Slicing…
          </span>
        {:else}
          Slice on server
        {/if}
      </Button>
    </div>
  {:else}
    <div class="space-y-3">
      {#each models as model (model.id)}
        <button
          type="button"
          class="flex w-full items-center space-x-4 rounded-lg border border-border p-3 text-left transition-all hover:border-primary hover:bg-muted"
          onclick={() => openInSlicer(model)}
        >
          <div
            class="flex h-14 w-14 shrink-0 items-center justify-center rounded bg-muted"
          >
            <File class="h-6 w-6 text-muted-foreground" />
          </div>
          <div class="min-w-0 flex-1 space-y-1">
            <h3 class="truncate font-medium">{model.name}</h3>
            <p class="text-sm text-muted-foreground">
              {toReadableSize(model.blob.size)} •
              {model.blob.filetype?.toUpperCase() || "Unknown"}
            </p>
          </div>
          <ChevronRight class="h-5 w-5 text-muted-foreground" />
        </button>
      {/each}
    </div>
  {/if}
</div>
