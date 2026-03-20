<script lang="ts">
  import GroupGrid from "$lib/components/view/group-grid.svelte";
  import { page } from "$app/state";
  import EditLabel from "$lib/components/edit/label.svelte";
  import type { Label } from "$lib/api/shared/label_api";
  import { sidebarState } from "$lib/sidebar_data.svelte";
  import { GroupStreamManager, IGroupApi } from "$lib/api/shared/group_api";
  import { getContainer } from "$lib/api/dependency_injection";

  let groupApi = getContainer().require<IGroupApi>(IGroupApi);

  let thisLabelOnly = $derived.by(() => {
    return page.url.searchParams.get("thisLabelOnly") === "true";
  });

  let label: Label | null = $derived.by(() => {
    let slug = parseInt(page.params.slug!);
    return sidebarState.labels.find((label) => label.meta.id === slug) ?? null;
  });
</script>

{#if label}
  <div class="flex h-full w-full flex-col">
    <EditLabel class="mx-4 my-3" {label} onDelete={() => (label = null)} />
    <div class="h-full overflow-hidden">
      <GroupGrid
        groupStream={new GroupStreamManager(
          groupApi,
          null,
          (thisLabelOnly ? [label.meta] : label.effectiveLabels).map(
            (x) => x.id,
          ),
          true,
        )}
      />
    </div>
  </div>
{:else}
  <div class="flex h-full w-full flex-col justify-center">
    <h1 class="mx-auto font-bold">Label not found</h1>
  </div>
{/if}
