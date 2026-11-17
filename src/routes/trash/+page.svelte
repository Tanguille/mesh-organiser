<script lang="ts">
  import { getContainer } from "#lib/api/dependency_injection";
  import { IModelApi, type Model } from "#lib/api/shared/model_api";
  import {
    IGroupApi,
    type DeletedGroup,
    type GroupRestoreRequest,
  } from "#lib/api/shared/group_api";
  import { ISettingsApi, SettingSection } from "#lib/api/shared/settings_api";
  import { Button } from "#lib/components/ui/button/index.js";
  import { Checkbox } from "#lib/components/ui/checkbox/index.js";
  import ModelTinyList from "#lib/components/view/model-tiny-list.svelte";
  import Spinner from "#lib/components/view/spinner.svelte";
  import {
    configuration,
    configurationMeta,
    currentUser,
  } from "#lib/configuration.svelte";
  import { updateSidebarState } from "#lib/sidebar_data.svelte";
  import { countWriter, timeSinceDate } from "#lib/utils";
  import ArchiveRestore from "@lucide/svelte/icons/archive-restore";
  import Trash2 from "@lucide/svelte/icons/trash-2";
  import Folder from "@lucide/svelte/icons/folder";
  import { onMount } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { toast } from "svelte-sonner";

  const modelApi = getContainer().require<IModelApi>(IModelApi);
  const groupApi = getContainer().optional<IGroupApi>(IGroupApi);
  const settingsApi = getContainer().optional<ISettingsApi>(ISettingsApi);
  // The retention is a desktop setting. An online account's trash lives on the
  // server, whose retention the client cannot read.
  const retentionKnown =
    !!settingsApi?.availableSections().includes(SettingSection.Behaviour) &&
    !currentUser.permissions.onlineAccount;

  let models = $state<Model[]>([]);
  let groups = $state<DeletedGroup[]>([]);
  let loading = $state(true);
  let loadFailed = $state(false);
  let modelLoadFailed = $state(false);
  let groupLoadFailed = $state(false);
  let restorableGroups = $derived(groups.filter(canRestoreGroup));
  const selected = new SvelteSet<number>();
  const selectedGroups = new SvelteSet<number>();

  onMount(async () => {
    const [modelResult, groupResult] = await Promise.allSettled([
      modelApi.getTrashedModels(),
      groupApi?.getTrashedGroups?.() ?? Promise.resolve([]),
    ]);
    if (modelResult.status === "fulfilled") models = modelResult.value;
    else modelLoadFailed = true;
    if (groupResult.status === "fulfilled") groups = groupResult.value;
    else groupLoadFailed = true;
    loadFailed = modelLoadFailed || groupLoadFailed;
    if (loadFailed) {
      toast.error("Failed to load the trash");
    }
    loading = false;
  });

  function canRestoreGroup(
    group: DeletedGroup,
  ): group is DeletedGroup & GroupRestoreRequest {
    return !!group.deletionOperationId;
  }

  function selectAll(checked: boolean) {
    selected.clear();
    selectedGroups.clear();
    if (checked) {
      models.forEach((model) => selected.add(model.id));
      restorableGroups.forEach((group) => selectedGroups.add(group.id));
    }
  }

  async function restoreSelected() {
    const restored = models.filter((model) => selected.has(model.id));
    const restoredGroups = restorableGroups.filter((group) =>
      selectedGroups.has(group.id),
    );
    // Clearing first disables the button, so a double click cannot restore twice.
    selected.clear();
    selectedGroups.clear();

    if (restored.length) {
      const promise = modelApi.restoreModels(restored);

      toast.promise(promise, {
        loading: `Restoring ${countWriter("model", restored)}...`,
        success: (_) => {
          return `Restored ${countWriter("model", restored)}`;
        },
        error: () =>
          "Failed to restore models. Refresh the trash and try again.",
      });

      // The toast reports a failure. Refetch either way instead of filtering
      // locally: a refused batch, or a restore made elsewhere (sync, MCP, another
      // tab), would otherwise leave stale rows that fail every retry.
      const restoreSucceeded = await promise.then(
        () => true,
        () => false,
      );
      try {
        models = await modelApi.getTrashedModels();
      } catch {
        // A successful restore must not leave live models available for another restore.
        if (restoreSucceeded) {
          const restoredIds = new Set(restored.map((model) => model.id));
          models = models.filter((model) => !restoredIds.has(model.id));
        }
        toast.error(
          "Failed to refresh the trash. Reload the page to try again.",
        );
      }
      const remaining = new Set(models.map((model) => model.id));
      for (const id of selected) {
        if (!remaining.has(id)) {
          selected.delete(id);
        }
      }
    }
    if (
      restoredGroups.length &&
      groupApi?.restoreGroups &&
      groupApi.getTrashedGroups
    ) {
      const succeeded = await groupApi.restoreGroups(restoredGroups).then(
        () => true,
        () => {
          toast.error("Failed to restore groups. Try again.");
          return false;
        },
      );
      try {
        groups = await groupApi.getTrashedGroups();
      } catch {
        if (succeeded) {
          const ids = new Set(restoredGroups.map((group) => group.id));
          groups = groups.filter((group) => !ids.has(group.id));
        }
        toast.error(
          "Failed to refresh the trash. Reload the page to try again.",
        );
      }
    }
    await updateSidebarState();
  }
</script>

{#snippet retention()}
  {#if retentionKnown}
    <p class="text-sm text-muted-foreground">
      Items older than {configuration.trash_retention_days}
      {configuration.trash_retention_days === 1 ? "day" : "days"} are permanently
      deleted the next time the app starts.
    </p>
  {/if}
{/snippet}

{#if !loading && (models.length > 0 || groups.length > 0)}
  <div class="flex h-full flex-col gap-3 px-5 py-3">
    {#if modelLoadFailed}<p role="alert">Could not load deleted models.</p>{/if}
    {#if groupLoadFailed}<p role="alert">Could not load deleted groups.</p>{/if}
    <div class="flex flex-row flex-wrap items-center gap-3 px-3">
      {#if !configurationMeta.applicationReadOnly}<Checkbox
          aria-label="Select all"
          disabled={models.length + restorableGroups.length === 0}
          bind:checked={
            () =>
              models.length + restorableGroups.length > 0 &&
              selected.size + selectedGroups.size ===
                models.length + restorableGroups.length,
            (checked) => selectAll(checked)
          }
        />{/if}
      <span class="text-sm">
        {selected.size + selectedGroups.size > 0
          ? `${selected.size + selectedGroups.size} selected`
          : groups.length === 0
            ? countWriter("model", models)
            : `${models.length + groups.length} items`}
      </span>
      <div class="grow">
        {@render retention()}
      </div>
      {#if !configurationMeta.applicationReadOnly}<Button
          disabled={selected.size + selectedGroups.size === 0}
          onclick={restoreSelected}
        >
          <ArchiveRestore /> Restore
        </Button>{/if}
    </div>

    <div class="hide-scrollbar flex flex-col gap-2 overflow-y-auto">
      {#each models as model (model.id)}
        {@const isSelected = selected.has(model.id)}
        <label class="flex cursor-pointer flex-row items-center gap-2">
          {#if !configurationMeta.applicationReadOnly}<Checkbox
              aria-label="Select {model.name}"
              bind:checked={
                () => isSelected,
                (checked) =>
                  checked ? selected.add(model.id) : selected.delete(model.id)
              }
            />{/if}
          <ModelTinyList
            {model}
            class="h-16 flex-1 {isSelected ? 'border-primary' : ''}"
          >
            {#snippet subtitle()}
              {#if model.deleted}
                <p
                  class="text-xs text-muted-foreground"
                  title={model.deleted.toLocaleString()}
                >
                  Deleted {timeSinceDate(model.deleted)}
                </p>
              {/if}
            {/snippet}
          </ModelTinyList>
        </label>
      {/each}
      {#each groups as group (group.id)}
        {@const isSelected = selectedGroups.has(group.id)}
        <label class="flex cursor-pointer flex-row items-center gap-2">
          {#if !configurationMeta.applicationReadOnly}<Checkbox
              aria-label="Select group {group.name}"
              disabled={!canRestoreGroup(group)}
              bind:checked={
                () => isSelected,
                (checked) =>
                  checked
                    ? selectedGroups.add(group.id)
                    : selectedGroups.delete(group.id)
              }
            />{/if}
          <div
            class="flex min-h-16 flex-1 items-center gap-3 rounded-lg border p-3 {isSelected
              ? 'border-primary'
              : ''}"
          >
            <Folder />
            <div>
              <p>{group.name}</p>
              {#if !canRestoreGroup(group)}
                <p class="text-xs text-muted-foreground">
                  This group cannot be restored automatically. Its models remain
                  in your library or Trash.
                </p>
              {/if}
              <p
                class="text-xs text-muted-foreground"
                title={group.deleted.toLocaleString()}
              >
                Deleted {timeSinceDate(group.deleted)}
              </p>
            </div>
          </div>
        </label>
      {/each}
    </div>
  </div>
{:else if loading}
  <div
    role="status"
    aria-label="Loading the trash"
    class="flex h-full w-full items-center justify-center"
  >
    <Spinner />
  </div>
{:else}
  <div
    class="flex h-full w-full flex-col items-center justify-center gap-2 text-center"
  >
    <div class="mb-2 rounded-md bg-primary p-2">
      <Trash2 />
    </div>
    {#if loadFailed}
      Could not load the trash.
    {:else}
      The trash is empty. Deleted models and groups show up here until they are
      restored or permanently deleted.
    {/if}
    {@render retention()}
  </div>
{/if}
