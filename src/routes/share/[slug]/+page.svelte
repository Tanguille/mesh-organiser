<script lang="ts">
  import { page } from "$app/state";
  import { getContainer } from "#lib/api/dependency_injection";
  import {
    getGroupsOfModels,
    GroupOrderBy,
    IGroupApi,
    PredefinedGroupStreamManager,
    type Group,
  } from "#lib/api/shared/group_api";
  import { IShareApi, type Share } from "#lib/api/shared/share_api";
  import GroupGrid from "#lib/components/view/group-grid.svelte";
  import Spinner from "#lib/components/view/spinner.svelte";
  import Share2 from "@lucide/svelte/icons/share-2";
  import { onMount } from "svelte";

  const groupApi = getContainer().require<IGroupApi>(IGroupApi);
  const shareApi = getContainer().require<IShareApi>(IShareApi);

  let shareId = $derived(page.params.slug!);
  let share = $state<Share | null>(null);
  let groups = $state<Group[]>([]);
  let loadStatus = $state<"loading" | "failed" | "done">("loading");

  onMount(async () => {
    try {
      share = await shareApi.getShare(shareId);
      groups = await getGroupsOfModels(
        groupApi,
        share.modelIds,
        GroupOrderBy.CreatedAsc,
      );
      loadStatus = "done";
    } catch (e) {
      loadStatus = "failed";
      console.error("Failed to load share", e);
    }
  });
</script>

{#if groups.length > 0}
  <div class="flex h-full w-full flex-col">
    <div
      class="mx-auto my-1 w-fit rounded-md bg-secondary px-4 py-2 text-secondary-foreground"
    >
      <p class="text-xl">{share?.shareName}, by user {share?.userName}</p>
    </div>
    <div class="h-full overflow-hidden">
      <GroupGrid
        groupStream={new PredefinedGroupStreamManager(groups)}
        default_show_multiselect_all={true}
      />
    </div>
  </div>
{:else if loadStatus === "loading"}
  <div
    role="status"
    aria-label="Loading the share"
    class="flex h-full w-full items-center justify-center"
  >
    <Spinner />
  </div>
{:else}
  <div
    class="flex h-full w-full flex-col items-center justify-center gap-2 text-center"
  >
    <div class="mb-2 rounded-md bg-primary p-2">
      <Share2 />
    </div>
    {#if loadStatus === "failed"}
      Could not load this share.
    {:else}
      This share has no models.
    {/if}
  </div>
{/if}
