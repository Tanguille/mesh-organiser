<script lang="ts">
  import { resolve } from "#lib/paths";
  import { goto } from "$app/navigation";
  import {
    createGroupMetaInstance,
    type Group,
    type GroupMeta,
    type IGroupStreamManager,
  } from "#lib/api/shared/group_api";
  import type { Model } from "#lib/api/shared/model_api";
  import type { LabelMeta } from "#lib/api/shared/label_api";
  import type { FileType } from "#lib/api/shared/blob_api";
  import {
    convertOrderOptionGroupsToEnum,
    GROUP_ORDER_LABELS,
  } from "#lib/api/shared/settings_api";
  import { SizeOptionClasses } from "#lib/components/view/size-classes";
  import EditGroup from "#lib/components/edit/group.svelte";
  import ModelEdit from "#lib/components/edit/model.svelte";
  import EditMultiModel from "#lib/components/edit/multi-model.svelte";
  import Checkbox from "#lib/components/ui/checkbox/checkbox.svelte";
  import DragSelectedModels from "#lib/components/view/drag-selected-models.svelte";
  import FileTypeFilter from "#lib/components/view/file-type-filter.svelte";
  import { createGridSelection } from "#lib/components/view/grid-selection.svelte";
  import ModelGridInner from "#lib/components/view/model-grid-inner.svelte";
  import RightClickModels from "#lib/components/view/right-click-models.svelte";
  import SearchInput from "#lib/components/view/search-input.svelte";
  import SortFilter from "#lib/components/view/sort-filter.svelte";
  import EmptyState from "#lib/components/view/empty-state.svelte";
  import UiSizeFilter from "#lib/components/view/ui-size-filter.svelte";
  import { configuration, configurationMeta } from "#lib/configuration.svelte";
  import { trashModels } from "#lib/components/view/trash-models";
  import { onDestroy, untrack } from "svelte";
  import { MediaQuery } from "svelte/reactivity";
  import { type ClassValue } from "svelte/elements";
  import GroupTinyList from "./group-tiny-list.svelte";
  import GroupTiny from "./group-tiny.svelte";
  import { uniqueById, wait } from "#lib/utils";
  import { IsMobile } from "#lib/hooks/is-mobile.svelte";
  import { buttonVariants } from "../ui/button/button.svelte";
  import SelectionPanel from "#lib/components/view/selection-panel.svelte";
  import Spinner from "#lib/components/view/spinner.svelte";

  const props: {
    groupStream: IGroupStreamManager;
    default_show_multiselect_all?: boolean;
  } = $props();
  let loadedGroups = $state<Group[]>([]);
  let selected = $state.raw<Group[]>([]);
  let trashing = false;
  let renameRequested = $state(0);
  let splitRenameRequested = $state(0);

  function onRename(group: Group) {
    if (
      !configurationMeta.applicationReadOnly &&
      selected.length === 1 &&
      selected[0].meta.id === group.meta.id
    )
      renameRequested++;
  }

  function onSplitRename(model: Model) {
    if (
      !configurationMeta.applicationReadOnly &&
      splitViewSelectedModels.length === 1 &&
      splitViewSelectedModels[0].id === model.id
    )
      splitRenameRequested++;
  }

  async function onTrash(models: Model[]) {
    if (
      trashing ||
      configurationMeta.applicationReadOnly ||
      models.length === 0
    )
      return;
    trashing = true;
    try {
      await trashModels($state.snapshot(models), onDelete);
    } finally {
      trashing = false;
    }
  }

  const isMobile = new IsMobile();
  const showLeftSide = $derived(!isMobile.current || selected.length <= 0);
  const showRightSide = $derived(!isMobile.current || selected.length > 0);

  // Below 1600px wide there is no room for the split view.
  const SPLIT_GRID_BREAKPOINT = 1600;
  const tooNarrowForSplit = new MediaQuery(
    `max-width: ${SPLIT_GRID_BREAKPOINT - 1}px`,
  );

  let effectiveSplitSetting = $derived.by(() => {
    if (tooNarrowForSplit.current) {
      return "no_split";
    }

    return configuration.group_split_view;
  });

  let scrollContainer: HTMLElement;
  let busyLoadingNext = $state.raw<boolean>(false);
  let loadFailed = $state(false);
  let initialLoadComplete = $state(false);
  let fileTypes = $state<FileType[]>([]);
  const hasFileTypeFilter = $derived(fileTypes.length > 0);
  let searchInput = $state.raw<{ clearSearch: () => void } | undefined>();
  let searchText = $state<string | null>(null);
  let loadGeneration = 0;

  async function fetchNextGroupSet() {
    if (busyLoadingNext || loadFailed) return;

    busyLoadingNext = true;
    const generation = loadGeneration;
    try {
      const newGroups = await props.groupStream.fetch();
      if (generation === loadGeneration) loadedGroups.push(...newGroups);
    } catch (error) {
      if (generation === loadGeneration) {
        loadFailed = true;
        console.error("Failed to load groups:", error);
      }
    } finally {
      busyLoadingNext = false;
      if (generation === loadGeneration) initialLoadComplete = true;
    }
  }

  async function resetGroupSet() {
    const generation = ++loadGeneration;
    loadedGroups = [];
    initialLoadComplete = false;
    pruneSelectedToLoaded();
    while (busyLoadingNext) {
      await wait(50);
    }

    if (generation !== loadGeneration) return;
    loadFailed = false;
    await fetchNextGroupSet();
    pruneSelectedToLoaded();
  }

  async function setNewSearchText(newText: string | null) {
    searchText = newText;
    props.groupStream.setSearchText(newText);
    await resetGroupSet();
  }

  function clearFilters() {
    fileTypes = [];
    props.groupStream.setFileTypes([]);
    searchInput?.clearSearch();
  }

  // Reuse the shared item size classes, layering on the image-strip widths the
  // list view needs for its `.imglist` container.
  const sizes = {
    ...SizeOptionClasses,
    List_Small: `${SizeOptionClasses.List_Small} [&_.imglist]:w-[115px]`,
    List_Medium: `${SizeOptionClasses.List_Medium} [&_.imglist]:w-[165px]`,
    List_Large: `${SizeOptionClasses.List_Large} [&_.imglist]:w-[235px]`,
  };

  const size = $derived(sizes[configuration.size_option_groups]);

  $effect(() => {
    props.groupStream.setOrderBy(
      convertOrderOptionGroupsToEnum(configuration.order_option_groups),
    );
  });

  let splitViewSelectedModels = $state.raw<Model[]>([]);

  function clearSplitViewModels() {
    splitViewSelectedModels = [];
  }

  function pruneSelectedToLoaded() {
    const ids = new Set(loadedGroups.map((g) => g.meta.id));
    const next = selected.filter((x) => ids.has(x.meta.id));
    if (next.length !== selected.length) {
      clearSplitViewModels();
    }
    selected = next;
  }

  // Selection interaction shared with model-grid-inner. The split view's
  // model sub-selection must be cleared whenever the group selection is about
  // to change, so it hooks in via onBeforeSelect. fetchNextGroupSet already
  // self-guards on busyLoadingNext.
  const selection = createGridSelection<Group>({
    getItems: () => loadedGroups,
    getSelected: () => selected,
    setSelected: (next) => {
      renameRequested = 0;
      splitRenameRequested = 0;
      selected = next;
    },
    getId: (group) => group.meta.id,
    onBeforeSelect: clearSplitViewModels,
    getScrollContainer: () => scrollContainer,
    onEndOfList: fetchNextGroupSet,
    onRename,
    onActivate: (group) => {
      if (group.meta.id < 0) return false;
      goto(resolve("/group/" + group.meta.id));
      return true;
    },
  });

  onDestroy(() => {
    selection.destroy();
  });

  $effect(() => {
    void loadedGroups;
    selection.reconcileFocus();
  });

  const modelsInSelectedGroups = $derived(selected.flatMap((g) => g.models));
  const modelsInLoadedGroups = $derived(loadedGroups.flatMap((g) => g.models));
  const onlyUngroupedModelsSelected = $derived(
    selected.length > 0 && selected.every((group) => group.meta.id < 0),
  );

  function onGroupKeyDown(group: Group, event: KeyboardEvent) {
    if (
      event.key === "Delete" &&
      !event.repeat &&
      !event.ctrlKey &&
      !event.metaKey &&
      !event.altKey &&
      !configurationMeta.applicationReadOnly &&
      onlyUngroupedModelsSelected
    ) {
      event.preventDefault();
      event.stopPropagation();
      void onTrash(modelsInSelectedGroups);
      return;
    }
    selection.onKeyDown(group, event);
  }

  let selectedModels = $derived(
    splitViewSelectedModels.length <= 0
      ? modelsInSelectedGroups
      : splitViewSelectedModels,
  );

  function updateGroupSummary(group: Group) {
    group.labels = uniqueById(group.models.flatMap((model) => model.labels));
    group.flags = {
      printed: group.models.some((model) => model.flags.printed),
      favorite: group.models.some((model) => model.flags.favorite),
    };
  }

  function onDelete(models: Model[]) {
    const ids = new Set(models.map((model) => model.id));
    loadedGroups = loadedGroups.filter((group) => {
      const affected = group.models.some((model) => ids.has(model.id));
      if (affected) {
        group.models = group.models.filter((model) => !ids.has(model.id));
        updateGroupSummary(group);
      }
      return !affected || group.models.length > 0;
    });
    splitViewSelectedModels = splitViewSelectedModels.filter(
      (model) => !ids.has(model.id),
    );
    pruneSelectedToLoaded();
  }

  function onGroupDelete(deletedGroup: Group, removeEmptyGroup = true) {
    const groupIndex = loadedGroups.findIndex(
      (g) => g.meta.id === deletedGroup.meta.id,
    );
    const deletedIds = new Set(deletedGroup.models.map((model) => model.id));
    if (groupIndex !== -1) {
      const group = loadedGroups[groupIndex];
      const removed = group.models.filter((model) => deletedIds.has(model.id));
      group.models = group.models.filter((model) => !deletedIds.has(model.id));
      updateGroupSummary(group);
      if (group.models.length === 0 && removeEmptyGroup)
        loadedGroups.splice(groupIndex, 1);
      [...removed].reverse().forEach((m) => {
        m.group = null;
        let newGroup: Group = {
          meta: createGroupMetaInstance(
            m.id * -1,
            m.name,
            m.added.toISOString(),
            m.lastModified.toISOString(),
            "",
          ),
          models: [m],
          labels: m.labels,
          flags: m.flags,
          resource: null,
        };
        loadedGroups.splice(groupIndex, 0, newGroup);
      });
    }

    splitViewSelectedModels = splitViewSelectedModels.filter(
      (model) => !deletedIds.has(model.id),
    );
    pruneSelectedToLoaded();
  }

  function onGroupDeleteViaModels(models: Model[]) {
    const ids = new Set(models.map((model) => model.id));
    const groups = loadedGroups.filter((group) => group.meta.id >= 0);
    for (const group of groups) {
      const affected = group.models.filter((model) => ids.has(model.id));
      if (affected.length > 0)
        onGroupDelete({ ...group, models: affected }, false);
    }
  }

  function onModelsMoved(models: Model[], destination: GroupMeta) {
    const ids = new Set(models.map((model) => model.id));
    const currentModels = loadedGroups
      .flatMap((group) => group.models)
      .filter((model) => ids.has(model.id));
    const destinationGroup = loadedGroups.find(
      (group) => group.meta.id === destination.id,
    );
    for (const model of currentModels) model.group = destination;
    loadedGroups = loadedGroups.filter((group) => {
      if (group === destinationGroup) return true;
      const count = group.models.length;
      group.models = group.models.filter((model) => !ids.has(model.id));
      updateGroupSummary(group);
      return (
        group.meta.id >= 0 ||
        count === group.models.length ||
        group.models.length > 0
      );
    });
    if (destinationGroup) {
      const existing = new Set(
        destinationGroup.models.map((model) => model.id),
      );
      destinationGroup.models.push(
        ...currentModels.filter((model) => !existing.has(model.id)),
      );
      updateGroupSummary(destinationGroup);
    }
    splitViewSelectedModels = splitViewSelectedModels.filter(
      (model) => !ids.has(model.id),
    );
    pruneSelectedToLoaded();
  }

  $effect(() => {
    const moved = (event: Event) => {
      const detail = (
        event as CustomEvent<{ models: Model[]; group: GroupMeta }>
      ).detail;
      onModelsMoved(detail.models, detail.group);
    };
    window.addEventListener("models-moved", moved);
    const labelled = (event: Event) => {
      const { models, label } = (
        event as CustomEvent<{ models: Model[]; label: LabelMeta }>
      ).detail;
      const ids = new Set(models.map((model) => model.id));
      for (const group of loadedGroups) {
        let affected = false;
        for (const model of group.models) {
          if (
            ids.has(model.id) &&
            !model.labels.some((item) => item.id === label.id)
          ) {
            model.labels.push(label);
            affected = true;
          }
        }
        if (affected && !group.labels.some((item) => item.id === label.id))
          group.labels.push(label);
      }
    };
    window.addEventListener("models-labelled", labelled);
    return () => {
      window.removeEventListener("models-moved", moved);
      window.removeEventListener("models-labelled", labelled);
    };
  });

  $effect(() => {
    void props.groupStream;
    untrack(() => resetGroupSet());
  });
</script>

<div class="flex h-full flex-row">
  {#if showLeftSide}
    <div class="flex flex-1 flex-col gap-1" style="min-width: 0;">
      <div class="flex flex-row justify-center gap-3 px-5 py-3">
        <SearchInput bind:this={searchInput} onsearch={setNewSearchText} />

        <FileTypeFilter
          bind:value={fileTypes}
          onchange={(x) => {
            props.groupStream.setFileTypes(x);
            resetGroupSet();
          }}
        />

        <SortFilter
          bind:value={configuration.order_option_groups}
          options={GROUP_ORDER_LABELS}
          onchange={(x) => {
            props.groupStream.setOrderBy(convertOrderOptionGroupsToEnum(x));
            resetGroupSet();
          }}
        />

        <UiSizeFilter bind:value={configuration.size_option_groups} />
      </div>

      {#if selected.length > 0}
        <div
          class="flex flex-wrap justify-between gap-2 px-5 text-xs text-muted-foreground"
        >
          <span aria-live="polite">{selected.length} selected</span>
          <span
            >Ctrl/⌘+A select loaded · {configurationMeta.applicationReadOnly
              ? ""
              : "F2 rename · "}Enter open</span
          >
        </div>
      {/if}
      {#if effectiveSplitSetting === "no_split"}
        {@render GroupGrid()}
      {:else if effectiveSplitSetting === "split-left-right"}
        {@render SplitView(
          "grid h-full grid-cols-[1fr_auto_1fr] gap-3 overflow-hidden",
          "border-l border-dashed",
          undefined,
        )}
      {:else if effectiveSplitSetting === "split-top-bottom"}
        {@render SplitView(
          "flex h-full flex-col gap-3 overflow-hidden",
          "border-t border-dashed",
          "h-full",
        )}
      {/if}
      {#if loadFailed}
        <button type="button" onclick={() => setNewSearchText(searchText)}>
          Failed to load groups. Retry
        </button>
      {:else if !initialLoadComplete && loadedGroups.length === 0}
        <div role="status" class="flex items-center justify-center gap-2">
          <Spinner /> Loading groups
        </div>
      {:else if loadedGroups.length === 0}
        <EmptyState>
          {#if searchText || hasFileTypeFilter}
            No matching groups
            <button type="button" class="ml-2 underline" onclick={clearFilters}
              >Clear filters</button
            >
          {:else}
            No groups found. <a href={resolve("/import")} class="underline"
              >Import models</a
            >
          {/if}
        </EmptyState>
      {/if}
    </div>
  {/if}

  {#if showRightSide}
    <SelectionPanel
      onClose={() => {
        clearSplitViewModels();
        selected = [];
      }}
    >
      {#if selected.length >= 2}
        {@render SelectedModelsEditor()}
      {:else if selected.length === 1 && selected[0].meta.id >= 0}
        <EditGroup
          group={selected[0]}
          settingsVertical={true}
          {renameRequested}
          onDelete={onGroupDelete}
        />
        {#if effectiveSplitSetting === "no_split"}
          <a
            class={buttonVariants({ variant: "default" })}
            href={resolve("/group/" + selected[0].meta.id)}>View models</a
          >
        {/if}
        {#if selected[0].models.length >= 2}
          {@render SelectedModelsEditor()}
        {:else if selected[0].models.length === 1}
          <ModelEdit
            model={selected[0].models[0]}
            {onDelete}
            {onModelsMoved}
            renameRequested={splitRenameRequested}
            onGroupDelete={onGroupDeleteViaModels}
          />
        {:else}
          <EmptyState>No models in group</EmptyState>
        {/if}
      {:else if selected.length === 1}
        <ModelEdit
          model={selected[0].models[0]}
          {onDelete}
          {onModelsMoved}
          {renameRequested}
          onGroupDelete={onGroupDeleteViaModels}
        />
      {:else if props.default_show_multiselect_all && loadedGroups.length > 0}
        <EditMultiModel
          models={modelsInLoadedGroups}
          {onDelete}
          {onModelsMoved}
          onGroupDelete={onGroupDeleteViaModels}
        />
      {:else}
        <EmptyState>No group selected</EmptyState>
      {/if}
    </SelectionPanel>
  {/if}
</div>

{#snippet SelectedModelsEditor()}
  {#if selectedModels.length >= 2}
    <EditMultiModel
      models={selectedModels}
      {onDelete}
      {onModelsMoved}
      onGroupDelete={onGroupDeleteViaModels}
    />
  {:else if selectedModels.length === 1}
    <ModelEdit
      model={selectedModels[0]}
      {onDelete}
      {onModelsMoved}
      renameRequested={splitRenameRequested}
      onGroupDelete={onGroupDeleteViaModels}
    />
  {/if}
{/snippet}

{#snippet SplitView(
  wrapperClass: ClassValue,
  dividerClass: ClassValue,
  innerClazz: ClassValue | undefined,
)}
  <span class={wrapperClass}>
    {@render GroupGrid()}
    <div class={dividerClass}></div>
    {#if selected.length >= 1}
      <ModelGridInner
        bind:value={
          () => splitViewSelectedModels,
          (value) => {
            splitRenameRequested = 0;
            splitViewSelectedModels = value;
          }
        }
        itemSize={configuration.size_option_groups}
        availableModels={modelsInSelectedGroups}
        clazz={innerClazz}
        {onTrash}
        {onModelsMoved}
        onRename={onSplitRename}
      />
    {:else}
      <EmptyState>No models in group to display</EmptyState>
    {/if}
  </span>
{/snippet}

{#snippet GroupGrid()}
  <div
    class="h-full overflow-y-scroll"
    role="listbox"
    aria-label="Groups"
    aria-multiselectable="true"
    bind:this={scrollContainer}
    onscroll={selection.handleScroll}
    onfocusout={selection.onFocusOut}
  >
    <DragSelectedModels models={modelsInSelectedGroups} class="select-none">
      <RightClickModels
        models={modelsInSelectedGroups}
        onTrash={onlyUngroupedModelsSelected ? onTrash : undefined}
        {onModelsMoved}
        class={`flex flex-row flex-wrap content-start justify-center gap-2 outline-0 ${configuration.show_multiselect_checkboxes && configuration.size_option_groups.includes("Grid") ? "pt-[5px]" : ""}`}
      >
        {#if configuration.size_option_groups.includes("List")}
          {#each loadedGroups as group (group.meta.id)}
            {@const isSelected = selection.selectedSet.has(group.meta.id)}
            <div class="grid w-full grid-cols-[auto_1fr] items-center gap-2">
              {@render GroupCheckbox(group, "", isSelected)}
              <div
                role="option"
                data-drag-type={group.meta.id >= 0 ? "group" : undefined}
                data-drag-param={group.meta.id >= 0 ? group.meta.id : undefined}
                tabindex={selection.tabIndex(group)}
                aria-selected={isSelected}
                oncontextmenu={(e) => selection.onRightClick(group, e)}
                onclick={(e) => selection.onClick(group, e)}
                onkeydown={(e) => onGroupKeyDown(group, e)}
                onfocus={() => selection.onFocus(group)}
                ondblclick={() => {
                  if (group.meta.id >= 0)
                    goto(resolve("/group/" + group.meta.id));
                }}
                onmousedown={(e) =>
                  selection.earlyOnClick(group, e, isSelected)}
                class="min-w-0 cursor-pointer"
              >
                <GroupTinyList
                  {group}
                  class="{size} pointer-events-none select-none {isSelected
                    ? 'border-primary'
                    : ''}"
                />
              </div>
            </div>
          {/each}
        {:else}
          {#each loadedGroups as group (group.meta.id)}
            {@const isSelected = selection.selectedSet.has(group.meta.id)}
            <div class="group relative">
              <div
                role="option"
                data-drag-type={group.meta.id >= 0 ? "group" : undefined}
                data-drag-param={group.meta.id >= 0 ? group.meta.id : undefined}
                tabindex={selection.tabIndex(group)}
                aria-selected={isSelected}
                oncontextmenu={(e) => selection.onRightClick(group, e)}
                onclick={(e) => selection.onClick(group, e)}
                onkeydown={(e) => onGroupKeyDown(group, e)}
                onfocus={() => selection.onFocus(group)}
                ondblclick={() => {
                  if (group.meta.id >= 0)
                    goto(resolve("/group/" + group.meta.id));
                }}
                onmousedown={(e) =>
                  selection.earlyOnClick(group, e, isSelected)}
                class="cursor-pointer"
              >
                <GroupTiny
                  {group}
                  class="{size} pointer-events-none select-none {isSelected
                    ? 'border-primary'
                    : ''}"
                />
              </div>
              {@render GroupCheckbox(
                group,
                `absolute top-[-5px] left-[-5px] bg-card rounded-lg ${isSelected ? "" : "group-hover:opacity-100 opacity-0"}`,
                isSelected,
              )}
            </div>
          {/each}
        {/if}
      </RightClickModels>
    </DragSelectedModels>
  </div>
{/snippet}

{#snippet GroupCheckbox(group: Group, clazz: ClassValue, isSelected: boolean)}
  {#if configuration.show_multiselect_checkboxes}
    <Checkbox
      class={clazz}
      aria-label="Select {group.meta.name}"
      tabindex={-1}
      bind:checked={() => isSelected, (val) => selection.toggle(group, val)}
    />
  {:else}
    <div></div>
  {/if}
{/snippet}
