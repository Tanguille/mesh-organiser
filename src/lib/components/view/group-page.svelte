<script lang="ts">
  import ModelGrid from "$lib/components/view/model-grid.svelte";
  import EditGroup from "$lib/components/edit/group.svelte";
  import {
    getGroupById,
    IGroupApi,
    type Group,
    type GroupMeta,
  } from "$lib/api/shared/group_api";
  import { onMount } from "svelte";
  import { getContainer } from "$lib/api/dependency_injection";
  import Spinner from "./spinner.svelte";
  import { PredefinedModelStreamManager } from "$lib/api/shared/model_api";

  interface Function {
    (): void;
  }

  const props: {
    group: GroupMeta;
    initialEditMode?: boolean;
    onGroupDelete?: Function;
    onAllModelsDelete?: Function;
  } = $props();
  let group = $state<Group | null>(null);
  let loading = $state(true);

  onMount(async () => {
    const groupApi = getContainer().require<IGroupApi>(IGroupApi);
    group = await getGroupById(groupApi, props.group.id);
    loading = false;
  });
</script>

{#if group}
  <div class="flex h-full w-full flex-col">
    <EditGroup
      class="mx-4 my-3"
      {group}
      onDelete={() => props.onGroupDelete?.()}
    />
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
{:else}
  <div class="flex h-full w-full flex-col justify-center">
    <h1 class="mx-auto font-bold">Group not found</h1>
  </div>
{/if}
