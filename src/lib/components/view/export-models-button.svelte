<script lang="ts">
  import { getContainer } from "$lib/api/dependency_injection";
  import { ILocalApi } from "$lib/api/shared/local_api";
  import { downloadModels, IDownloadApi } from "$lib/api/shared/download_api";
  import type { Model } from "$lib/api/shared/model_api";
  import FolderOpen from "@lucide/svelte/icons/folder-open";
  import ChevronDown from "@lucide/svelte/icons/chevron-down";
  import Download from "@lucide/svelte/icons/download";
  import * as DropdownMenu from "$lib/components/ui/dropdown-menu/index.js";
  import { AsyncButton, Button } from "$lib/components/ui/button/index.js";
  import Package from "@lucide/svelte/icons/package";
  import Boxes from "@lucide/svelte/icons/boxes";

  const localApi = getContainer().optional<ILocalApi>(ILocalApi);
  const downloadApi = getContainer().optional<IDownloadApi>(IDownloadApi);
  const props: { models: Model[]; class?: string } = $props();
  let busy = $state(false);

  async function openInFolder(asZip: boolean) {
    busy = true;
    try {
      await localApi?.openInFolder(props.models, asZip);
    } finally {
      busy = false;
    }
  }
</script>

{#if localApi}
  <div class="flex flex-row {props.class}">
    <Button
      class="grow rounded-r-none"
      disabled={busy}
      onclick={() => openInFolder(false)}
    >
      <FolderOpen /> Open in folder
    </Button>
    <DropdownMenu.Root>
      <DropdownMenu.Trigger disabled={busy}>
        {#snippet child({ props })}
          <Button {...props} class="rounded-l-none px-1">
            <ChevronDown />
          </Button>
        {/snippet}
      </DropdownMenu.Trigger>
      <DropdownMenu.Content align="end" class="w-56">
        <DropdownMenu.Item onclick={() => openInFolder(false)}>
          <Boxes /> Export as individual models
        </DropdownMenu.Item>
        <DropdownMenu.Item onclick={() => openInFolder(true)}>
          <Package /> Export as .zip file
        </DropdownMenu.Item>
      </DropdownMenu.Content>
    </DropdownMenu.Root>
  </div>
{:else if downloadApi}
  <AsyncButton
    class={props.class}
    onclick={() => downloadModels(props.models, downloadApi)}
  >
    <Download /> Download {props.models.length > 1 ? "models" : "model"}
  </AsyncButton>
{/if}
