<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { getContainer } from '$lib/api/dependency_injection';
  import { IWebImportApi } from '$lib/api/shared/web_import_api';
  import { Button } from '$lib/components/ui/button/index.js';
  import { Spinner } from '$lib/components/view/spinner.svelte';
  import { Card, CardHeader, CardTitle, CardContent } from '$lib/components/ui/card';
  import { CheckboxWithLabel } from '$lib/components/ui/checkbox/index';
  import { globalImportSettings } from '$lib/import.svelte';
  import { LoaderCircle } from "@lucide/svelte/icons/loader-circle";
  import File from "@lucide/svelte/icons/file";
  import { countWriter } from '$lib/utils';
  import { writable } from 'svelte/store';

  // Import state store
  const importState = writable({
    status: 'Idle' as const, // Idle | ProcessingThumbnails | Finished | Failure
    current_importing_group: '',
    model_count: 0,
    imported_models_count: 0,
    finished_thumbnails_count: 0,
    imported_models: [] as any[],
    failure_reason: ''
  });

  let dialog_open = false;
  let webImportApi;

  onMount(async () => {
    webImportApi = getContainer().optional<IWebImportApi>(IWebImportApi);
  });

  async function handleWebImport() {
    dialog_open = true;
    try {
      await webImportApi?.openFilesForImporting();
    } finally {
      dialog_open = false;
    }
  }

  function resetImport() {
    // Reset import state
    importState.set({
      status: 'Idle' as const,
      current_importing_group: '',
      model_count: 0,
      imported_models_count: 0,
      finished_thumbnails_count: 0,
      imported_models: [],
      failure_reason: ''
    });
  }

  $: importStatus = $importState.status;
</script>

<div class="flex h-full justify-center">
  {#if importStatus == 'Finished'}
    <div class="flex w-full flex-col gap-1">
      <div class="mt-4 flex flex-row justify-center gap-5">
        <Button onclick={resetImport}
          ><LoaderCircle class="h-4 w-4 mr-2 animate-spin" /> Import another model</Button
        >
        <div class="my-auto">
          Imported {countWriter(
            "group",
            $importState.imported_models.filter((g: any) => g.meta.id >= 0)
          )}, {countWriter("model", $importState.imported_models.flatMap((g: any) => g.models))}
        </div>
      </div>
      {:if $importState.imported_models.length > 0}
        <div class="w-full grow overflow-hidden">
          {/* Would display imported models here - for now just show a message */}
          <div class="text-center py-8">
            <p class="text-lg font-medium">Import successful!</p>
            <p class="text-muted">Models are now available in your library.</p>
            <Button on:click={() => goto('/')} class="mt-4">
              Go to Library
            </Button>
          </div>
        </div>
      {:else}
        <div class="flex h-full w-full items-center justify-center">
          <Spinner />
        </div>
      {/if}
    </div>
  {:else if importStatus == 'Idle'}
    <div class="max-w-xl my-auto flex h-fit flex-col gap-5">
      {#if webImportApi}
        <Card>
          <CardHeader>
            <CardTitle>Import Models</CardTitle>
            <CardDescription>Import 3D models from files</CardDescription>
          </CardHeader>
          <CardContent class="flex flex-col gap-4">
            <div class="grid grid-cols-2 gap-4">
              <Button
                class="grow"
                onclick={handleWebImport}
                disabled={dialog_open}
              ><File /> Import Files</Button
              >
            </div>

            <div
              class="flex h-[150px] w-full items-center justify-center rounded-md border border-dashed text-sm"
            >
              <p>Drag and drop files here</p>
            </div>

            <CheckboxWithLabel
              label="Delete files after import"
              bind:value={globalImportSettings.delete_after_import}
            />
          </CardContent>
        </Card>
      {:else}
        <div class="text-center py-8">
          <p class="text-lg font-medium text-muted">Import functionality not available</p>
        </div>
      {/if}
    </div>
  {:else if importStatus == 'Failure'}
    <div class="my-auto flex flex-col items-center gap-4">
      <h1>Import failed</h1>
      <p class="text-sm">
        An error occurred during the import process. Please try again.
      </p>
      <p class="text-sm">{$importState.failure_reason}</p>
      <Button onclick={resetImport} class="mt-4"><LoaderCircle class="h-4 w-4 mr-2 animate-spin" /> Go back</Button>
    </div>
  {:else}
    <div class="my-auto flex flex-col items-center gap-2">
      {#if $importState.current_importing_group}
        <h1>Group: {$importState.current_importing_group}</h1>
      {/if}
      {#if importStatus == 'ProcessingThumbnails'}
        <h1>
          Generated {$importState.finished_thumbnails_count}/{$importState.model_count}
          thumbnails...
        </h1>
      {:else if $importState.imported_models_count > 0}
        <h1>
          Imported {$importState.imported_models_count}/{$importState.model_count} models...
        </h1>
      {:else}
        <h1>Importing model...</h1>
      {/if}
      <div>
        <LoaderCircle class="h-10 w-10 animate-spin" />
      </div>
    </div>
  {/if}
</div>

<style>
  .model-import-screen {
    padding: 1rem;
  }
  
  .loading, .error, .empty {
    text-align: center;
    padding: 2rem;
    color: var(--text-muted);
  }
  
  .error {
    color: var(--error);
  }
</style>