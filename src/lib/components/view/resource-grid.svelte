<script lang="ts">
  import { resolve } from "$lib/paths";
  import { Input } from "$lib/components/ui/input";
  import * as Select from "$lib/components/ui/select/index.js";
  import GroupTinyList from "./group-tiny-list.svelte";
  import { AsyncButton, buttonVariants } from "$lib/components/ui/button";
  import EditResource from "$lib/components/edit/resource.svelte";
  import NotebookText from "@lucide/svelte/icons/notebook-text";
  import ClipboardCheck from "@lucide/svelte/icons/clipboard-check";
  import Button from "../ui/button/button.svelte";
  import {
    IResourceApi,
    type ResourceMeta,
  } from "$lib/api/shared/resource_api";
  import { getContainer } from "$lib/api/dependency_injection";
  import type { Group } from "$lib/api/shared/group_api";
  import { updateSidebarState } from "$lib/sidebar_data.svelte";
  import { ILocalApi } from "$lib/api/shared/local_api";
  import { configuration } from "$lib/configuration.svelte";
  import OpenInSlicerButton from "./open-in-slicer-button.svelte";
  import { IDownloadApi } from "$lib/api/shared/download_api";
  import { handleGridItemKeyDown } from "$lib/utils";
  import Download from "@lucide/svelte/icons/download";
  import ExportModelsButton from "./export-models-button.svelte";
  import { downloadModelsWithToast } from "./grid-helpers";

  const props: { resources: ResourceMeta[] } = $props();
  let selected = $state.raw<ResourceMeta | null>(null);
  let groups = $state.raw<Group[]>([]);
  let newName = $state<string>("");

  const resourceApi = getContainer().require<IResourceApi>(IResourceApi);
  const localApi = getContainer().optional<ILocalApi>(ILocalApi);
  const downloadApi = getContainer().optional<IDownloadApi>(IDownloadApi);

  let scrollContainer: HTMLElement;

  interface SearchFilters {
    search: string;
    order: "date-asc" | "date-desc" | "name-asc" | "name-desc";
    limit: number;
  }

  const currentFilter = $state<SearchFilters>({
    search: "",
    order: "date-desc",
    limit: 100,
  });

  function handleScroll() {
    if (scrollContainer && currentFilter.limit < filteredCollection.length) {
      const { scrollTop, scrollHeight, clientHeight } = scrollContainer;
      if (scrollTop + clientHeight >= scrollHeight) {
        currentFilter.limit += 100;
      }
    }
  }

  const readableOrders = {
    "date-asc": "Date (Asc)",
    "date-desc": "Date (Desc)",
    "name-asc": "Name (Asc)",
    "name-desc": "Name (Desc)",
  };

  const readableOrder = $derived(readableOrders[currentFilter.order]);

  const filteredCollection = $derived.by(() => {
    let search_lower = currentFilter.search.toLowerCase();

    return props.resources
      .filter((resource) => resource.name.toLowerCase().includes(search_lower))
      .sort((a, b) => {
        switch (currentFilter.order) {
          case "date-asc":
            return (
              new Date(a.created).getTime() - new Date(b.created).getTime()
            );
          case "date-desc":
            return (
              new Date(b.created).getTime() - new Date(a.created).getTime()
            );
          case "name-asc":
            return a.name.localeCompare(b.name);
          case "name-desc":
            return b.name.localeCompare(a.name);
          default:
            return 0;
        }
      });
  });

  async function onClick(
    resource: ResourceMeta,
    event: MouseEvent | KeyboardEvent,
  ) {
    selected = resource;

    const el = event.target;
    if (el instanceof HTMLElement) {
      setTimeout(() => {
        el.scrollIntoView({ behavior: "smooth", block: "center" });
      }, 30);
    }

    groups = await resourceApi.getGroupsForResource(resource);
  }

  async function onKeyDown(resource: ResourceMeta, event: KeyboardEvent) {
    handleGridItemKeyDown(resource, event, onClick, false);
  }

  async function onNewResource() {
    const newResource = await resourceApi.addResource(newName);
    props.resources.push(newResource);
    selected = newResource;
    await updateSidebarState();
  }

  async function onDownloadModel(group: Group) {
    if (!downloadApi) {
      return;
    }

    await downloadModelsWithToast(downloadApi, group.models);
  }

  async function deleteResource(resource: ResourceMeta) {
    props.resources.splice(props.resources.indexOf(resource!), 1);
    selected = null;
    await updateSidebarState();
  }
</script>

<div class="flex h-full flex-row">
  <div class="flex flex-1 flex-col gap-1" style="min-width: 0;">
    <div class="grid grid-cols-2 justify-center gap-5 px-5 py-3">
      <Input
        bind:value={currentFilter.search}
        class="border-primary"
        placeholder="Search"
      />

      <Select.Root type="single" name="Sort" bind:value={currentFilter.order}>
        <Select.Trigger class="border-primary">
          {readableOrder}
        </Select.Trigger>
        <Select.Content>
          <Select.Group>
            <Select.GroupHeading>Sort options</Select.GroupHeading>
            {#each Object.entries(readableOrders) as order (order[0])}
              <Select.Item value={order[0]} label={order[1]}
                >{order[1]}</Select.Item
              >
            {/each}
          </Select.Group>
        </Select.Content>
      </Select.Root>
    </div>

    <div
      class="flex h-full flex-row flex-wrap content-start gap-2 overflow-y-scroll outline-0"
      bind:this={scrollContainer}
      onscroll={handleScroll}
    >
      {#each filteredCollection.slice(0, currentFilter.limit) as resource (resource.id)}
        {@const isSelected = selected?.id === resource.id}
        <div
          role="option"
          tabindex="0"
          aria-selected={isSelected}
          oncontextmenu={(e) => onClick(resource, e)}
          onclick={(e) => onClick(resource, e)}
          onkeydown={(e) => onKeyDown(resource, e)}
          class="flex h-14 w-full min-w-0 cursor-pointer flex-row gap-3 overflow-hidden rounded-lg border p-1 px-3 select-none [&_.imglist]:w-[165px] {isSelected
            ? 'border-primary'
            : ''}"
        >
          {#if resource.flags.completed}
            <ClipboardCheck class="h-full" />
          {:else}
            <NotebookText class="h-full" />
          {/if}

          <div class="my-auto h-fit flex-1 overflow-hidden">
            <h2 class="truncate font-bold">{resource.name}</h2>
            {#if configuration.show_date_on_list_view}
              <p class="hidden-if-small ml-4 text-xs font-thin">
                Created {resource.created.toLocaleDateString()}
              </p>
            {/if}
          </div>
        </div>
      {/each}
    </div>

    <div class="grid grid-cols-3 justify-center gap-5 px-5 py-3">
      <Input
        bind:value={newName}
        class="col-span-2 border-primary"
        placeholder="New placeholder name..."
      />
      <Button onclick={onNewResource} disabled={newName.length <= 0}
        >Create project</Button
      >
    </div>
  </div>
  <div
    class="relative mx-4 my-2 hide-scrollbar flex w-[400px] min-w-[400px] flex-col gap-4 overflow-y-auto"
  >
    {#if !!selected}
      <EditResource
        resource={selected}
        onDelete={(_) => deleteResource(selected!)}
      />

      {#each groups as group (group.meta.id)}
        <div class="grid grid-cols-1 gap-2 rounded-lg border pt-1">
          <GroupTinyList
            {group}
            class="h-14 w-full border-none [&_.imglist]:w-[165px]"
          />
          <a
            href={resolve("/group/" + group.meta.id)}
            class="mx-3 {buttonVariants({ variant: 'default' })}"
          >
            Open group
          </a>
          <div class="mx-3 mt-2 mb-4 grid grid-cols-2 gap-4">
            {#if localApi}
              <ExportModelsButton models={group.models} class="grow" />
            {:else if downloadApi}
              <AsyncButton class="grow" onclick={() => onDownloadModel(group)}
                ><Download /> Download model</AsyncButton
              >
            {/if}

            <OpenInSlicerButton models={group.models} class="grow" />
          </div>
        </div>
      {/each}
      <div
        class="flex flex-col items-center justify-center gap-4 rounded-md border border-dashed p-4"
      >
        <span>Add groups to projects in the groups menu</span>
        <a
          href={resolve("/group")}
          class="w-full {buttonVariants({ variant: 'secondary' })}"
          >Go to the groups menu</a
        >
      </div>
    {:else}
      <div
        class="flex h-full flex-col items-center justify-center rounded-md border border-dashed"
      >
        <span class="text-xl">No project selected</span>
      </div>
    {/if}
  </div>
</div>
