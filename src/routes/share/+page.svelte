<script lang="ts">
  import { getContainer } from "$lib/api/dependency_injection";
  import { IShareApi, type Share } from "$lib/api/shared/share_api";
  import ShareEdit from "$lib/components/edit/share.svelte";
  import { onMount } from "svelte";
  import Share2 from "@lucide/svelte/icons/share-2";

  const shareApi = getContainer().require<IShareApi>(IShareApi);
  let shares = $state<Share[]>([]);
  let loading = $state<boolean>(true);

  onMount(async () => {
    shares = await shareApi.getShares();
    loading = false;
  });
</script>

{#if shares.length >= 1}
  <div class="hide-scrollbar h-full w-full overflow-y-auto">
    <div
      class="fix-card-width relative my-3 flex flex-row flex-wrap justify-center gap-5"
    >
      {#each shares as share (share.id)}
        <ShareEdit
          {share}
          class="w-[500px]"
          onDelete={() => (shares = shares.filter((x) => x != share))}
        />
      {/each}
    </div>
  </div>
{:else if !loading}
  <div class="flex h-full w-full flex-col items-center justify-center">
    <div class="color-primary-foreground mb-4 rounded-md bg-primary p-2">
      <Share2 />
    </div>
    No shares available. Share models via the share option in the model and group
    menu's.
  </div>
{/if}
