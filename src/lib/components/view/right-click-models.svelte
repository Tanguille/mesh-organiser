<script lang="ts">
  import * as ContextMenu from "#lib/components/ui/context-menu/index.js";
  import FolderOpen from "@lucide/svelte/icons/folder-open";
  import Slice from "@lucide/svelte/icons/slice";
  import GroupIcon from "@lucide/svelte/icons/group";
  import type { ClassValue } from "svelte/elements";
  import { goto } from "$app/navigation";
  import { resolve } from "#lib/paths";
  import type { Model } from "#lib/api/shared/model_api";
  import { getContainer } from "#lib/api/dependency_injection";
  import { ISlicerApi } from "#lib/api/shared/slicer_api";
  import { ILocalApi } from "#lib/api/shared/local_api";
  import { createShare, IShareApi } from "#lib/api/shared/share_api";
  import Share2 from "@lucide/svelte/icons/share-2";
  import { FileType } from "#lib/api/shared/blob_api";
  import {
    extractThreemfModels,
    IThreemfApi,
  } from "#lib/api/shared/threemf_api";
  import { IGroupApi, type GroupMeta } from "#lib/api/shared/group_api";
  import GroupSelect from "#lib/components/view/group-select.svelte";
  import type { Snippet } from "svelte";
  import PackageOpen from "@lucide/svelte/icons/package-open";
  import { countWriter } from "#lib/utils";
  import Trash2 from "@lucide/svelte/icons/trash-2";
  import { configurationMeta } from "#lib/configuration.svelte";

  const props: {
    children: Snippet;
    models: Model[];
    class?: ClassValue;
    onTrash?: (models: Model[]) => void;
    onModelsMoved?: (models: Model[], group: GroupMeta) => void;
  } = $props();
  const shareApi = getContainer().optional<IShareApi>(IShareApi);
  const slicerApi = getContainer().optional<ISlicerApi>(ISlicerApi);
  const localApi = getContainer().optional<ILocalApi>(ILocalApi);
  const threemfApi = getContainer().optional<IThreemfApi>(IThreemfApi);
  const groupApi = getContainer().optional<IGroupApi>(IGroupApi);
  const group = $derived.by(() => {
    if (props.models.length <= 0) {
      return null;
    }

    const firstGroup = props.models[0].group;

    if (!firstGroup || firstGroup.id <= 0) {
      return null;
    }

    return props.models.every((model) => model.group?.id === firstGroup.id)
      ? firstGroup!
      : null;
  });

  async function onOpenInSlicer() {
    let slicerApi = getContainer().optional<ISlicerApi>(ISlicerApi);
    if (slicerApi) {
      await slicerApi.openInSlicer(props.models);
    }
  }

  async function onOpenInFolder() {
    let localApi = getContainer().optional<ILocalApi>(ILocalApi);

    if (localApi) {
      await localApi.openInFolder(props.models, false);
    }
  }

  async function onOpenGroup() {
    if (group?.id) {
      goto(resolve("/group/" + group.id));
    }
  }
</script>

<ContextMenu.Root>
  <ContextMenu.Trigger class="{props.class} relative">
    {@render props.children?.()}
  </ContextMenu.Trigger>
  <ContextMenu.Content class="w-64">
    <ContextMenu.Item inset disabled
      >Selected {countWriter("model", props.models)}</ContextMenu.Item
    >
    {#if props.models.length > 0}
      {#if slicerApi}
        <ContextMenu.Item inset onclick={onOpenInSlicer}
          ><Slice class="mr-2 size-5" /> Open in slicer</ContextMenu.Item
        >
      {/if}
      {#if localApi}
        <ContextMenu.Item inset onclick={onOpenInFolder}
          ><FolderOpen class="mr-2 size-5" /> Open in folder</ContextMenu.Item
        >
      {/if}
      {#if props.onTrash && !configurationMeta.applicationReadOnly}
        <ContextMenu.Item inset onclick={() => props.onTrash?.(props.models)}>
          <Trash2 class="mr-2 size-5" /> Move to trash
        </ContextMenu.Item>
      {/if}
      {#if groupApi && !configurationMeta.applicationReadOnly}
        <GroupSelect
          models={props.models}
          onModelsMoved={props.onModelsMoved}
          menuItem
        />
      {/if}
    {/if}
    {#if group}
      <ContextMenu.Item inset onclick={onOpenGroup}
        ><GroupIcon class="mr-2 size-5" />
        <span class="flex-1 truncate">Open group '{group.name}'</span
        ></ContextMenu.Item
      >
    {/if}
    {#if shareApi && props.models.length > 0}
      <ContextMenu.Item
        inset
        onclick={async () => createShare(props.models, shareApi)}
        ><Share2 class="mr-2 size-5" /> Share selected models</ContextMenu.Item
      >
    {/if}
    {#if props.models.length === 1 && props.models[0].blob.filetype == FileType.THREEMF && threemfApi}
      <ContextMenu.Item
        inset
        onclick={async () =>
          extractThreemfModels(props.models[0], threemfApi, groupApi)}
        ><PackageOpen class="mr-2 size-5" /> Extract models from 3MF</ContextMenu.Item
      >
    {/if}
  </ContextMenu.Content>
</ContextMenu.Root>
