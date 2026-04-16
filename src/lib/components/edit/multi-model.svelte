<script lang="ts">
  import {
    Card,
    CardContent,
    CardHeader,
    CardTitle,
  } from "$lib/components/ui/card";

  import { CheckboxWithLabel } from "$lib/components/ui/checkbox/index.js";
  import { Label } from "$lib/components/ui/label";
  import LabelSelect from "$lib/components/view/label-select.svelte";
  import { countWriter } from "$lib/utils";

  import { goto } from "$app/navigation";
  import { resolve } from "$lib/paths";

  import {
    AsyncButton,
    Button,
    buttonVariants,
  } from "$lib/components/ui/button/index.js";
  import * as DropdownMenu from "$lib/components/ui/dropdown-menu/index.js";
  import Ellipsis from "@lucide/svelte/icons/ellipsis";
  import type { ClassValue } from "svelte/elements";

  import { getContainer } from "$lib/api/dependency_injection";
  import { type GroupMeta, IGroupApi } from "$lib/api/shared/group_api";
  import { ILabelApi, type LabelMeta } from "$lib/api/shared/label_api";
  import { ILocalApi } from "$lib/api/shared/local_api";
  import { IModelApi, type Model } from "$lib/api/shared/model_api";
  import { sidebarState, updateSidebarState } from "$lib/sidebar_data.svelte";
  import Boxes from "@lucide/svelte/icons/boxes";
  import Component from "@lucide/svelte/icons/component";
  import Group from "@lucide/svelte/icons/group";
  import Trash2 from "@lucide/svelte/icons/trash-2";
  import Ungroup from "@lucide/svelte/icons/ungroup";
  import { IDownloadApi } from "$lib/api/shared/download_api";
  import Download from "@lucide/svelte/icons/download";
  import OpenInSlicerButton from "../view/open-in-slicer-button.svelte";
  import { createShare, IShareApi } from "$lib/api/shared/share_api";
  import Share2 from "@lucide/svelte/icons/share-2";
  import { configurationMeta } from "$lib/configuration.svelte";
  import ExportModelsButton from "../view/export-models-button.svelte";
  import { downloadModelsWithToast } from "../view/grid-helpers";
  import { toast } from "svelte-sonner";

  interface Function {
    (): void;
  }

  const props: {
    models: Model[];
    class?: ClassValue;
    onDelete?: Function;
    onGroupDelete?: Function;
  } = $props();

  const models = $derived(props.models);
  const printed = $derived(models.every((x) => x.flags.printed));
  const favorited = $derived(models.every((x) => x.flags.favorite));
  const allModelGroups = $derived.by(() => {
    const seen = new Set<number>();
    const result: GroupMeta[] = [];
    for (const m of models) {
      const g = m.group;
      if (g && !seen.has(g.id)) {
        seen.add(g.id);
        result.push(g);
      }
    }
    return result;
  });
  const availableGroups = $derived(
    allModelGroups.filter((g) => !models.every((x) => x.group?.id === g.id)),
  );

  const availableLabels = $derived(sidebarState.labels.map((l) => l.meta));
  const appliedLabels = $derived.by(() => {
    const seen = new Set<number>();
    const result: LabelMeta[] = [];
    for (const m of models) {
      for (const l of m.labels) {
        if (!seen.has(l.id)) {
          seen.add(l.id);
          result.push(l);
        }
      }
    }
    return result;
  });

  const modelApi = getContainer().require<IModelApi>(IModelApi);
  const groupApi = getContainer().require<IGroupApi>(IGroupApi);
  const labelApi = getContainer().require<ILabelApi>(ILabelApi);
  const localApi = getContainer().optional<ILocalApi>(ILocalApi);
  const downloadApi = getContainer().optional<IDownloadApi>(IDownloadApi);
  const shareApi = getContainer().optional<IShareApi>(IShareApi);

  async function setLabelOnAllModels(label: LabelMeta) {
    const affectedModels = models;

    affectedModels.forEach((model) => model.labels.push(label));

    let promise = labelApi.addLabelToModels(
      label,
      $state.snapshot(affectedModels),
    );

    toast.promise(promise, {
      loading: `Adding label ${label.name} to ${countWriter("model", affectedModels)}...`,
      success: (_) => {
        return `Added label ${label.name} to ${countWriter("model", affectedModels)}`;
      },
    });

    await promise;
    await updateSidebarState();
  }

  async function removeLabelFromAllModels(label: LabelMeta) {
    const affectedModels = models;

    affectedModels.forEach(
      (model) =>
        (model.labels = model.labels.filter((entry) => entry.id !== label.id)),
    );

    let promise = labelApi.removeLabelFromModels(
      label,
      $state.snapshot(affectedModels),
    );

    toast.promise(promise, {
      loading: `Removing label ${label.name} from ${countWriter("model", affectedModels)}...`,
      success: (_) => {
        return `Removed label ${label.name} from ${countWriter("model", affectedModels)}`;
      },
    });

    await promise;
    await updateSidebarState();
  }

  async function setPrintedFlagOnAllModels(printed: boolean) {
    await setFlagOnAllModels((x) => (x.flags.printed = printed), printed);
  }

  async function setFavoriteFlagOnAllModels(favorite: boolean) {
    await setFlagOnAllModels((x) => (x.flags.favorite = favorite), favorite);
  }

  async function setFlagOnAllModels(action: (m: Model) => void, set: boolean) {
    const setOrUnset = set ? "Set" : "Unset";
    const affectedModels = models;

    affectedModels.forEach(action);

    const promise = (async () => {
      for (const model of affectedModels) {
        await modelApi.editModel($state.snapshot(model));
      }
    })();

    toast.promise(promise, {
      loading: `${setOrUnset}ting flag on ${countWriter("model", affectedModels)}...`,
      success: (_) => {
        return `${setOrUnset} flag on ${countWriter("model", affectedModels)}`;
      },
    });

    await promise;
    await updateSidebarState();
  }

  async function onAddModelsToGroup(group: GroupMeta) {
    const affectedModels = models;

    await groupApi.addModelsToGroup(group, $state.snapshot(affectedModels));
    await updateSidebarState();

    toast.success(
      `Added ${countWriter("model", affectedModels)} to group '${group.name}'`,
      {
        action: {
          label: "Go to group",
          onClick: () => {
            goto(resolve("/group/" + group.id));
          },
        },
      },
    );
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

  async function onDownloadModels() {
    if (!downloadApi) {
      return;
    }

    await downloadModelsWithToast(downloadApi, models);
  }

  async function onNewGroup() {
    const affectedModels = models;

    const newGroup = await groupApi.addGroup("New group");
    await groupApi.addModelsToGroup(newGroup, affectedModels);

    for (const model of affectedModels) {
      model.group = newGroup;
    }

    await updateSidebarState();

    goto(resolve("/group/" + newGroup.id));
  }

  async function onRemoveGroup() {
    const count = models.length;
    await groupApi.removeModelsFromGroup(models);
    for (const model of models) {
      model.group = null;
    }
    await updateSidebarState();
    toast.success(`Ungrouped ${count} model(s)`);
    props.onGroupDelete?.();
  }

  async function onDelete() {
    const affectedModels = models;

    let promise = modelApi.deleteModels(affectedModels);

    toast.promise(promise, {
      loading: `Deleting ${countWriter("model", affectedModels)}...`,
      success: (_) => {
        return `Deleted ${countWriter("model", affectedModels)}`;
      },
    });

    await promise;
    await updateSidebarState();
    props.onDelete?.();
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
          class="widthhack h-full"
          variant="ghost"
          onclick={() => createShare(models, shareApi)}
        >
          <Share2 />
        </Button>
      {/if}
      {#if !configurationMeta.applicationReadOnly}
        <DropdownMenu.Root>
          <DropdownMenu.Trigger>
            <div
              class="{buttonVariants({ variant: 'ghost' })} widthhack h-full"
            >
              <Ellipsis />
            </div>
          </DropdownMenu.Trigger>
          <DropdownMenu.Content side="right" align="start">
            <DropdownMenu.Item onclick={onDelete}>
              <Trash2 /> Delete selected models
            </DropdownMenu.Item>
          </DropdownMenu.Content>
        </DropdownMenu.Root>
      {/if}
    </CardHeader>
    <CardContent class="flex flex-col gap-8">
      <div class="flex flex-col gap-4">
        <Label>Open</Label>
        <div class="grid grid-cols-2 gap-4">
          {#if localApi}
            <ExportModelsButton {models} class="grow" />
          {:else if downloadApi}
            <AsyncButton class="grow" onclick={onDownloadModels}
              ><Download /> Download {models.length > 1
                ? "models"
                : "model"}</AsyncButton
            >
          {/if}
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
          <DropdownMenu.Root>
            <DropdownMenu.Trigger
              disabled={availableGroups.length <= 0}
              class="{buttonVariants({ variant: 'default' })} grow"
            >
              <Component /> Add selected to group
            </DropdownMenu.Trigger>
            <DropdownMenu.Content
              side="bottom"
              align="start"
              class="w-(--bits-dropdown-menu-anchor-width)"
            >
              {#each availableGroups as group (group.id)}
                <DropdownMenu.Item onclick={() => onAddModelsToGroup(group)}>
                  <Boxes class="mr-2" />
                  {group.name}
                </DropdownMenu.Item>
              {/each}
            </DropdownMenu.Content>
          </DropdownMenu.Root>
        </div>
        <div class="flex flex-col gap-4">
          <Label>Properties</Label>
          <CheckboxWithLabel
            class="ml-1"
            label="Printed"
            bind:value={() => printed, (val) => setPrintedFlagOnAllModels(val)}
          />
          <CheckboxWithLabel
            class="ml-1"
            label="Favorite"
            bind:value={
              () => favorited, (val) => setFavoriteFlagOnAllModels(val)
            }
          />
        </div>
      {/if}
    </CardContent>
  </Card>
{/if}
