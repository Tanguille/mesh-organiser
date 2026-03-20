<script lang="ts">
  import { Progress } from "$lib/components/ui/progress/index.js";
  import { importState } from "$lib/import.svelte";
  import * as Card from "$lib/components/ui/card/index.js";
  import LoaderCircle from "@lucide/svelte/icons/loader-circle";
  import CircleCheck from "@lucide/svelte/icons/circle-check";
  import CircleX from "@lucide/svelte/icons/circle-x";
  import { ImportStatus } from "$lib/api/shared/tauri_import_api";

  const state = $derived.by(() => {
    switch (importState.status) {
      case ImportStatus.Idle:
        return "Idle";
      case ImportStatus.ProcessingModels:
        return "Processing models...";
      case ImportStatus.FinishedModels:
        return "Finished processing models.";
      case ImportStatus.ProcessingThumbnails:
        return "Generating thumbnails...";
      case ImportStatus.FinishedThumbnails:
        return "Finished generating thumbnails.";
      case ImportStatus.Finished:
        return "Import finished";
      case ImportStatus.Failure:
        return "Import failed";
      default:
        return "Unknown status";
    }
  });

  const progress = $derived.by(() => {
    if (
      importState.status == ImportStatus.Idle ||
      importState.status == ImportStatus.Finished ||
      importState.status == ImportStatus.Failure
    ) {
      return 100;
    } else if (
      importState.status == ImportStatus.ProcessingModels ||
      importState.status == ImportStatus.FinishedModels
    ) {
      return Math.round(
        (importState.imported_models_count / importState.model_count) * 100,
      );
    } else {
      return Math.round(
        (importState.finished_thumbnails_count / importState.model_count) * 100,
      );
    }
  });
</script>

<Card.Root class="w-full">
  <Card.Header
    class="expanded-text-parent @container flex flex-row items-center justify-center gap-2 px-1 py-2"
  >
    {#if importState.status == ImportStatus.Finished}
      <CircleCheck />
    {:else if importState.status == ImportStatus.Failure}
      <CircleX />
    {:else}
      <div>
        <LoaderCircle class="w-full animate-spin" />
      </div>
    {/if}

    <div class="expanded-text m-0! h-full truncate text-sm @max-[50px]:hidden!">
      {state}
    </div>
  </Card.Header>
  <Card.Content class="px-1 pt-0 pb-2">
    <Progress value={progress} />
  </Card.Content>
</Card.Root>
