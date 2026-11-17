<script lang="ts">
  import ModelGrid from "#lib/components/view/model-grid.svelte";
  import EditGroup from "#lib/components/edit/group.svelte";
  import {
    getGroupById,
    IGroupApi,
    type Group,
    type GroupMeta,
  } from "#lib/api/shared/group_api";
  import { getContainer } from "#lib/api/dependency_injection";
  import Spinner from "./spinner.svelte";
  import { PredefinedModelStreamManager } from "#lib/api/shared/model_api";

  const props: {
    group: Pick<GroupMeta, "id">;
    initialEditMode?: boolean;
    onGroupDelete?: () => void;
    onAllModelsDelete?: () => void;
  } = $props();
  let group = $state<Group | null>(null);
  let loading = $state(true);
  let loadFailed = $state(false);
  let retryCount = $state(0);

  $effect(() => {
    const groupId = props.group.id;
    void retryCount;
    let current = true;
    const groupApi = getContainer().require<IGroupApi>(IGroupApi);
    group = null;
    loading = true;
    loadFailed = false;
    getGroupById(groupApi, groupId)
      .then((result) => {
        if (current) group = result;
      })
      .catch((error) => {
        if (current) {
          loadFailed = true;
          console.error("Failed to load group:", error);
        }
      })
      .finally(() => {
        if (current) loading = false;
      });
    return () => {
      current = false;
    };
  });
</script>

{#if group}
  <div class="flex h-full w-full flex-col">
    <div data-drag-type="group" data-drag-param={group.meta.id}>
      <EditGroup
        class="mx-4 my-3"
        {group}
        onDelete={() => props.onGroupDelete?.()}
      />
    </div>
    <div class="overflow-hidden">
      <ModelGrid
        initialEditMode={props.initialEditMode}
        onRemoveGroupDelete={true}
        modelStream={new PredefinedModelStreamManager(group.models)}
        default_show_multiselect_all={true}
        onEmpty={() => props.onAllModelsDelete?.()}
      />
    </div>
  </div>
{:else if loading}
  <div class="flex h-full w-full items-center justify-center">
    <Spinner />
  </div>
{:else if loadFailed}
  <div class="flex h-full w-full flex-col items-center justify-center">
    <p>Failed to load group.</p>
    <button type="button" onclick={() => retryCount++}>Retry</button>
  </div>
{:else}
  <div class="flex h-full w-full flex-col justify-center">
    <h1 class="mx-auto font-bold">Group not found</h1>
  </div>
{/if}
