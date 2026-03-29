<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { invoke, isTauri } from "@tauri-apps/api/core";
  import { getContainer } from "$lib/api/dependency_injection";
  import {
    IModelApi,
    ModelOrderBy,
    type Model,
  } from "$lib/api/shared/model_api";
  import {
    ISlicerApi,
    type SlicingSettings,
  } from "$lib/api/shared/slicer_api";
  import Spinner from "$lib/components/view/spinner.svelte";
  import Button from "$lib/components/ui/button/button.svelte";
  import { Label } from "$lib/components/ui/label/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import { ChevronRight, File, LoaderCircle } from "lucide-svelte";
  import { toast } from "svelte-sonner";

  const MATERIALS = ["PLA", "PETG", "ABS", "TPU", "ASA"] as const;

  let models: Model[] = $state([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let isMobileRemote = $state(false);
  let canSliceOnServer = $state(false);

  let selectedModelId = $state<number | null>(null);
  let settings = $state<SlicingSettings>({
    layerHeight: 0.2,
    infill: 20,
    supports: "none",
    material: "PLA",
  });
  let slicing = $state(false);

  async function detectRemoteSliceCapability() {
    if (!isTauri()) {
      isMobileRemote = false;
      canSliceOnServer = false;

      return;
    }

    try {
      isMobileRemote = await invoke<boolean>("is_mobile");
    } catch {
      isMobileRemote = false;
    }

    const slicerApi = getContainer().optional<ISlicerApi>(ISlicerApi);
    canSliceOnServer = typeof slicerApi?.sliceOnServer === "function";
  }

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

  async function onSliceOnServer() {
    if (selectedModelId === null) {
      toast.error("Select a model to slice.");

      return;
    }

    const slicerApi = getContainer().optional<ISlicerApi>(ISlicerApi);
    if (!slicerApi?.sliceOnServer) {
      toast.error("Server slicing is not available.");

      return;
    }

    const payload: SlicingSettings = {
      layerHeight: Number(settings.layerHeight),
      infill: Number(settings.infill),
      supports: settings.supports,
      material: settings.material,
    };

    slicing = true;
    try {
      const res = await slicerApi.sliceOnServer(selectedModelId, payload);
      if (res.success) {
        toast.success("Slice finished on server.", {
          description:
            res.message ??
            `Output model id ${res.outputBlobId}. G-code stays on the server.`,
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

  onMount(async () => {
    await detectRemoteSliceCapability();
    await loadModels();
  });
</script>

<div class="space-y-6 px-4 pt-4 pb-24">
  <div class="flex items-center justify-between">
    <h2 class="text-xl font-bold">
      {canSliceOnServer ? "Slice on server" : "Select a Model to Slice"}
    </h2>
    <p class="text-sm text-muted-foreground">
      {models.length} models available
    </p>
  </div>

  {#if canSliceOnServer}
    <p class="text-sm text-muted-foreground">
      G-code is generated on your Mesh Organiser server. Pick a model, adjust
      settings, then slice.
    </p>
  {:else if isMobileRemote}
    <p class="text-sm text-muted-foreground">
      This device is not connected to server slicing yet. Open a model and use
      <span class="font-medium">Open in slicer</span> from the model page, or
      configure your server in settings.
    </p>
  {:else}
    <p class="text-sm text-muted-foreground">
      Open a model to use <span class="font-medium">Open in slicer</span> with
      your desktop app.
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
        <Label for="slice-model">Model</Label>
        <div class="max-h-48 space-y-2 overflow-y-auto rounded-lg border border-border p-2">
          {#each models as model (model.id)}
            <button
              type="button"
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
                  {formatSize(model.blob.size)}
                </p>
              </div>
            </button>
          {/each}
        </div>
      </div>

      <div class="grid gap-4 sm:grid-cols-2">
        <div class="space-y-2">
          <Label for="layer-height">Layer height (mm)</Label>
          <select
            id="layer-height"
            class="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:outline-none"
            bind:value={settings.layerHeight}
          >
            <option value={0.1}>0.1</option>
            <option value={0.2}>0.2</option>
            <option value={0.3}>0.3</option>
          </select>
        </div>
        <div class="space-y-2">
          <Label for="infill">Infill (%)</Label>
          <Input
            id="infill"
            type="number"
            min="0"
            max="100"
            bind:value={settings.infill}
          />
        </div>
        <div class="space-y-2">
          <Label for="supports">Supports</Label>
          <select
            id="supports"
            class="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:outline-none"
            bind:value={settings.supports}
          >
            <option value="none">None</option>
            <option value="everywhere">Everywhere</option>
            <option value="touching buildplate">Touching build plate</option>
          </select>
        </div>
        <div class="space-y-2">
          <Label for="material">Material</Label>
          <select
            id="material"
            class="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:outline-none"
            bind:value={settings.material}
          >
            {#each MATERIALS as mat (mat)}
              <option value={mat}>{mat}</option>
            {/each}
          </select>
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

    {#if !isMobileRemote}
      <p class="text-sm text-muted-foreground">
        Open a model, then use <span class="font-medium text-foreground"
          >Open in slicer</span
        > to send it to your desktop slicer.
      </p>
    {/if}
  {/if}
</div>
