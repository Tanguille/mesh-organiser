<script lang="ts">
  import { getContainer } from "#lib/api/dependency_injection";
  import { IModelApi, type Model } from "#lib/api/shared/model_api";
  import { ISettingsApi, SettingSection } from "#lib/api/shared/settings_api";
  import { Button } from "#lib/components/ui/button/index.js";
  import { Checkbox } from "#lib/components/ui/checkbox/index.js";
  import ModelTinyList from "#lib/components/view/model-tiny-list.svelte";
  import { configuration, currentUser } from "#lib/configuration.svelte";
  import { updateSidebarState } from "#lib/sidebar_data.svelte";
  import { countWriter, isCommandError, timeSinceDate } from "#lib/utils";
  import ArchiveRestore from "@lucide/svelte/icons/archive-restore";
  import Trash2 from "@lucide/svelte/icons/trash-2";
  import { onMount } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { toast } from "svelte-sonner";

  const modelApi = getContainer().require<IModelApi>(IModelApi);
  const settingsApi = getContainer().optional<ISettingsApi>(ISettingsApi);
  // The retention is a desktop setting. An online account's trash lives on the
  // server, whose retention the client cannot read.
  const retentionKnown =
    !!settingsApi?.availableSections().includes(SettingSection.Behaviour) &&
    !currentUser.permissions.onlineAccount;

  let models = $state<Model[]>([]);
  let loading = $state(true);
  const selected = new SvelteSet<number>();

  onMount(async () => {
    models = await modelApi.getTrashedModels();
    loading = false;
  });

  function selectAll(checked: boolean) {
    selected.clear();
    if (checked) {
      models.forEach((model) => selected.add(model.id));
    }
  }

  async function restoreSelected() {
    const restored = models.filter((model) => selected.has(model.id));
    // Clearing first disables the button, so a double click cannot restore twice.
    selected.clear();

    const promise = modelApi.restoreModels(restored);

    toast.promise(promise, {
      loading: `Restoring ${countWriter("model", restored)}...`,
      success: (_) => {
        return `Restored ${countWriter("model", restored)}`;
      },
      error: (reason) =>
        isCommandError(reason)
          ? reason.error_inner_message
          : "Failed to restore models",
    });

    // The toast reports a failure. Refetch either way instead of filtering
    // locally: a refused batch, or a restore made elsewhere (sync, MCP, another
    // tab), would otherwise leave stale rows that fail every retry.
    await promise.catch(() => {});
    models = await modelApi.getTrashedModels();
    const remaining = new Set(models.map((model) => model.id));
    for (const id of selected) {
      if (!remaining.has(id)) {
        selected.delete(id);
      }
    }
    await updateSidebarState();
  }
</script>

{#snippet retention()}
  {#if retentionKnown}
    <p class="text-sm text-muted-foreground">
      Items are permanently deleted {configuration.trash_retention_days}
      {configuration.trash_retention_days === 1 ? "day" : "days"} after they are moved
      here.
    </p>
  {/if}
{/snippet}

{#if models.length > 0}
  <div class="flex h-full flex-col gap-3 px-5 py-3">
    <div class="flex flex-row flex-wrap items-center gap-3 px-3">
      <Checkbox
        aria-label="Select all"
        bind:checked={
          () => selected.size === models.length, (checked) => selectAll(checked)
        }
      />
      <span class="text-sm">
        {selected.size > 0
          ? `${selected.size} selected`
          : countWriter("model", models)}
      </span>
      <div class="grow">
        {@render retention()}
      </div>
      <Button disabled={selected.size === 0} onclick={restoreSelected}>
        <ArchiveRestore /> Restore
      </Button>
    </div>

    <div class="hide-scrollbar flex flex-col gap-2 overflow-y-auto">
      {#each models as model (model.id)}
        {@const isSelected = selected.has(model.id)}
        <label class="flex cursor-pointer flex-row items-center gap-2">
          <Checkbox
            aria-label="Select {model.name}"
            bind:checked={
              () => isSelected,
              (checked) =>
                checked ? selected.add(model.id) : selected.delete(model.id)
            }
          />
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
    </div>
  </div>
{:else if !loading}
  <div
    class="flex h-full w-full flex-col items-center justify-center gap-2 text-center"
  >
    <div class="mb-2 rounded-md bg-primary p-2">
      <Trash2 />
    </div>
    The trash is empty. Deleted models show up here until they are restored.
    {@render retention()}
  </div>
{/if}
