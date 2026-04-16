<script lang="ts">
  import * as Card from "$lib/components/ui/card/index.js";
  import LoaderCircle from "@lucide/svelte/icons/loader-circle";
  import { globalSyncState, SyncStage, SyncStep } from "$lib/sync.svelte";
  import Button from "../ui/button/button.svelte";
  import { currentUser } from "$lib/configuration.svelte";
  import { formatLastSyncedLabel } from "$lib/utils";
  import { getContainer } from "$lib/api/dependency_injection";
  import { ISyncApi } from "$lib/api/shared/sync_api";
  import RefreshCw from "@lucide/svelte/icons/refresh-cw";
  import { onDestroy } from "svelte";

  /** Idle: last-sync line; syncing: step + optional processed/total %. */
  function updateLastSync(): string {
    return formatLastSyncedLabel(currentUser.lastSync);
  }

  let lastSync = $state(updateLastSync());

  const stage = $derived.by(() => {
    switch (globalSyncState.stage) {
      case SyncStage.Models:
        return "Models";
      case SyncStage.Groups:
        return "Groups";
      case SyncStage.Labels:
        return "Labels";
      case SyncStage.Resources:
        return "Resources";
      default:
        return "";
    }
  });

  let step = $derived.by(() => {
    switch (globalSyncState.step) {
      case SyncStep.Upload:
        return `Uploading new ${stage}`;
      case SyncStep.Download:
        return `Downloading new ${stage}`;
      case SyncStep.UpdateMetadata:
        return `Updating metadata for ${stage}`;
      case SyncStep.Delete:
        return `Deleting ${stage}`;
      default:
        return "";
    }
  });

  async function onSyncClick() {
    const syncApi = getContainer().optional<ISyncApi>(ISyncApi);
    if (syncApi) {
      await syncApi.syncData();
    }
  }

  let tickTimer = setInterval(() => {
    lastSync = updateLastSync();
  }, 1000);

  let progress = $derived.by(() => {
    if (globalSyncState.stage == SyncStage.Idle) {
      return lastSync;
    } else if (
      globalSyncState.processedItems > 0 &&
      globalSyncState.processableItems > 0
    ) {
      return `${globalSyncState.processedItems}/${globalSyncState.processableItems} (${Math.round((globalSyncState.processedItems / globalSyncState.processableItems) * 100)}%)`;
    } else {
      return "";
    }
  });

  onDestroy(() => {
    clearInterval(tickTimer);
  });
</script>

<Card.Root class="w-full">
  <Card.Header
    class="expanded-text-parent @container flex flex-row items-center justify-center gap-2 px-1 py-2"
  >
    {#if globalSyncState.stage != SyncStage.Idle}
      <div>
        <LoaderCircle class="w-full animate-spin" />
      </div>

      <div
        class="expanded-text m-0! h-full truncate text-sm @max-[50px]:hidden!"
      >
        {step}
      </div>
    {:else}
      <Button
        class="expanded-text-parent @container w-full px-0"
        onclick={onSyncClick}
        ><RefreshCw /><span class="expanded-text @max-[50px]:hidden!"
          >Sync now</span
        ></Button
      >
    {/if}
  </Card.Header>
  {#if progress}
    <Card.Content class="expanded-text-parent @container px-1 pt-0 pb-2">
      <p class="expanded-text w-full text-center @max-[50px]:hidden!">
        {progress}
      </p>
    </Card.Content>
  {/if}
</Card.Root>
