<script lang="ts">
  import {
    Card,
    CardContent,
    CardHeader,
    CardTitle,
  } from "#lib/components/ui/card";

  import { CheckboxWithLabel } from "#lib/components/ui/checkbox/index.js";
  import { Label } from "#lib/components/ui/label";
  import LabelSelect from "#lib/components/view/label-select.svelte";
  import GroupSelect from "#lib/components/view/group-select.svelte";
  import { countWriter, uniqueById } from "#lib/utils";

  import { goto } from "$app/navigation";
  import { trashModels } from "#lib/components/view/trash-models";
  import { resolve } from "#lib/paths";

  import { Button, buttonVariants } from "#lib/components/ui/button/index.js";
  import * as DropdownMenu from "#lib/components/ui/dropdown-menu/index.js";
  import Ellipsis from "@lucide/svelte/icons/ellipsis";
  import { toast } from "svelte-sonner";
  import type { ClassValue } from "svelte/elements";

  import { getContainer } from "#lib/api/dependency_injection";
  import { type GroupMeta, IGroupApi } from "#lib/api/shared/group_api";
  import { ILabelApi, type LabelMeta } from "#lib/api/shared/label_api";
  import { IModelApi, type Model } from "#lib/api/shared/model_api";
  import { sidebarState, updateSidebarState } from "#lib/sidebar_data.svelte";
  import Group from "@lucide/svelte/icons/group";
  import Trash2 from "@lucide/svelte/icons/trash-2";
  import Ungroup from "@lucide/svelte/icons/ungroup";
  import OpenInSlicerButton from "../view/open-in-slicer-button.svelte";
  import { createShare, IShareApi } from "#lib/api/shared/share_api";
  import Share2 from "@lucide/svelte/icons/share-2";
  import { configurationMeta } from "#lib/configuration.svelte";
  import ExportModelsButton from "../view/export-models-button.svelte";

  const props: {
    models: Model[];
    class?: ClassValue;
    onDelete?: (models: Model[]) => void;
    onGroupDelete?: (models: Model[]) => void;
    onModelsMoved?: (models: Model[], group: GroupMeta) => void;
  } = $props();

  // This alias follows props.models; handlers capture the current array before
  // awaiting because the selection can change while a request is in flight.
  const models = $derived(props.models);
  let deleting = $state(false);
  const printed = $derived(models.every((x) => x.flags.printed));
  const favorited = $derived(models.every((x) => x.flags.favorite));
  const allModelGroups = $derived(
    uniqueById(models.map((x) => x.group).filter((g) => !!g)),
  );

  let availableLabels = $derived(sidebarState.labels.map((l) => l.meta));
  let appliedLabels = $derived(uniqueById(models.flatMap((x) => x.labels)));

  const modelApi = getContainer().require<IModelApi>(IModelApi);
  const groupApi = getContainer().require<IGroupApi>(IGroupApi);
  const labelApi = getContainer().require<ILabelApi>(ILabelApi);
  const shareApi = getContainer().optional<IShareApi>(IShareApi);

  async function setLabelOnAllModels(label: LabelMeta) {
    const affected_models = models;

    affected_models.forEach((x) => x.labels.push(label));

    let promise = labelApi.addLabelToModels(
      label,
      $state.snapshot(affected_models),
    );

    toast.promise(promise, {
      loading: `Adding label ${label.name} to ${countWriter("model", affected_models)}...`,
      success: (_) => {
        return `Added label ${label.name} to ${countWriter("model", affected_models)}`;
      },
    });

    await promise;
    await updateSidebarState();
  }

  async function removeLabelFromAllModels(label: LabelMeta) {
    const affected_models = models;

    affected_models.forEach(
      (x) => (x.labels = x.labels.filter((l) => l.id !== label.id)),
    );

    let promise = labelApi.removeLabelFromModels(
      label,
      $state.snapshot(affected_models),
    );

    toast.promise(promise, {
      loading: `Removing label ${label.name} from ${countWriter("model", affected_models)}...`,
      success: (_) => {
        return `Removed label ${label.name} from ${countWriter("model", affected_models)}`;
      },
    });

    await promise;
    await updateSidebarState();
  }

  async function setFlagOnAllModels(action: (m: Model) => void, set: boolean) {
    const set_or_unset = set ? "Set" : "Unset";
    const affected_models = models;

    affected_models.forEach(action);

    // Edit all models concurrently rather than awaiting one round-trip per model.
    let promise = Promise.all(
      affected_models.map((model) =>
        modelApi.editModel($state.snapshot(model)),
      ),
    );

    toast.promise(promise, {
      loading: `${set_or_unset}ting flag on ${countWriter("model", affected_models)}...`,
      success: (_) => {
        return `${set_or_unset} flag on ${countWriter("model", affected_models)}`;
      },
    });

    await promise;
    await updateSidebarState();
  }

  async function updateLabels(labels: LabelMeta[]) {
    const added_label = labels.find(
      (x) => !appliedLabels.some((l) => l.id === x.id),
    );
    const deleted_label = appliedLabels.find(
      (x) => !labels.some((l) => l.id === x.id),
    );

    if (added_label) {
      await setLabelOnAllModels(added_label);
    } else if (deleted_label) {
      await removeLabelFromAllModels(deleted_label);
    }
  }

  async function onNewGroup() {
    const affected_models = models;

    const newGroup = await groupApi.addGroup("New group");
    await groupApi.addModelsToGroup(newGroup, affected_models);

    for (const model of affected_models) {
      model.group = newGroup;
    }

    await updateSidebarState();

    goto(resolve("/group/" + newGroup.id));
  }

  async function onRemoveGroup() {
    const affected_models = $state.snapshot(models);
    await groupApi.removeModelsFromGroup(affected_models);
    props.onGroupDelete?.(affected_models);
    await updateSidebarState();
    toast.success(`Ungrouped ${countWriter("model", affected_models)}`);
  }

  async function onDelete() {
    if (deleting) return;
    const affected_models = $state.snapshot(models);
    deleting = true;
    try {
      await trashModels(affected_models, (trashed) =>
        props.onDelete?.(trashed),
      );
    } finally {
      deleting = false;
    }
  }
</script>

{#if models.length <= 0}
  No models to display
{:else}
  <Card class={props.class}>
    <CardHeader class="flex h-15 flex-row gap-2 space-y-0">
      <CardTitle class="my-auto h-fit">{countWriter("model", models)}</CardTitle
      >
      <div class="grow"></div>
      {#if shareApi}
        <Button
          class="h-full w-9"
          variant="ghost"
          onclick={() => createShare(models, shareApi)}
          aria-label="Share models"
        >
          <Share2 />
        </Button>
      {/if}
      {#if !configurationMeta.applicationReadOnly}
        <Button
          variant="ghost"
          class="h-full w-9"
          aria-label="Move to trash"
          disabled={deleting}
          onclick={onDelete}
        >
          <Trash2 />
        </Button>
        <DropdownMenu.Root>
          <DropdownMenu.Trigger aria-label="Model actions">
            <div class="{buttonVariants({ variant: 'ghost' })} h-full w-9">
              <Ellipsis />
            </div>
          </DropdownMenu.Trigger>
          <DropdownMenu.Content side="right" align="start">
            <DropdownMenu.Item onclick={onDelete} disabled={deleting}>
              <Trash2 /> Move selected models to trash
            </DropdownMenu.Item>
          </DropdownMenu.Content>
        </DropdownMenu.Root>
      {/if}
    </CardHeader>
    <CardContent class="flex flex-col gap-8">
      <div class="flex flex-col gap-4">
        <Label>Open</Label>
        <div class="grid grid-cols-2 gap-4">
          <ExportModelsButton {models} class="grow" />
          <OpenInSlicerButton {models} class="grow" />
        </div>
      </div>
      {#if !configurationMeta.applicationReadOnly}
        <div class="flex flex-col gap-4">
          <Label>Add/Remove labels</Label>

          <LabelSelect
            {availableLabels}
            bind:value={() => appliedLabels, (val) => updateLabels(val)}
          />
        </div>
        <div class="flex flex-col gap-4">
          <Label>Set/Unset group</Label>
          <div class="grid grid-cols-2 gap-4">
            <Button onclick={onNewGroup} class="grow"
              ><Group /> New group</Button
            >
            <Button
              onclick={onRemoveGroup}
              class="grow"
              disabled={allModelGroups.length <= 0}
              ><Ungroup /> Remove from group</Button
            >
          </div>
          <GroupSelect {models} onModelsMoved={props.onModelsMoved} />
        </div>
        <div class="flex flex-col gap-4">
          <Label>Properties</Label>
          <CheckboxWithLabel
            class="ml-1"
            label="Printed"
            bind:value={
              () => printed,
              (val) => setFlagOnAllModels((x) => (x.flags.printed = val), val)
            }
          />
          <CheckboxWithLabel
            class="ml-1"
            label="Favorite"
            bind:value={
              () => favorited,
              (val) => setFlagOnAllModels((x) => (x.flags.favorite = val), val)
            }
          />
        </div>
      {/if}
    </CardContent>
  </Card>
{/if}
