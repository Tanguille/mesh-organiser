<script lang="ts">
  import type { IModelStreamManager, Model } from "#lib/api/shared/model_api";
  import type { GroupMeta } from "#lib/api/shared/group_api";
  import type { LabelMeta } from "#lib/api/shared/label_api";
  import type { FileType } from "#lib/api/shared/blob_api";
  import {
    convertOrderOptionModelsToEnum,
    MODEL_ORDER_LABELS,
  } from "#lib/api/shared/settings_api";
  import ModelEdit from "#lib/components/edit/model.svelte";
  import MultiModelEdit from "#lib/components/edit/multi-model.svelte";
  import FileTypeFilter from "#lib/components/view/file-type-filter.svelte";
  import ModelGridInner from "#lib/components/view/model-grid-inner.svelte";
  import SearchInput from "#lib/components/view/search-input.svelte";
  import SortFilter from "#lib/components/view/sort-filter.svelte";
  import EmptyState from "#lib/components/view/empty-state.svelte";
  import UiSizeFilter from "#lib/components/view/ui-size-filter.svelte";
  import { configuration, configurationMeta } from "#lib/configuration.svelte";
  import { trashModels } from "#lib/components/view/trash-models";
  import { IsMobile } from "#lib/hooks/is-mobile.svelte";
  import { wait } from "#lib/utils";
  import { untrack } from "svelte";
  import SelectionPanel from "#lib/components/view/selection-panel.svelte";
  import Spinner from "#lib/components/view/spinner.svelte";
  import { resolve } from "#lib/paths";

  const props: {
    modelStream: IModelStreamManager;
    default_show_multiselect_all?: boolean;
    initialEditMode?: boolean;
    onRemoveGroupDelete?: boolean;
    onEmpty?: () => void;
  } = $props();
  let loadedModels = $state<Model[]>([]);
  let allModels = $state<Model[]>([]);
  let allModelsLoaded = false;
  let allModelsWithFallback = $derived(
    allModels.length > 0 ? allModels : loadedModels,
  );
  let selected = $state.raw<Model[]>([]);
  let busyLoadingNext = $state.raw<boolean>(false);
  let loadFailed = $state(false);
  let initialLoadComplete = $state(false);
  let searchText = $state<string | null>(null);
  let loadGeneration = 0;
  let hasFileTypeFilter = $state(false);
  let fileTypes = $state<FileType[]>([]);
  let searchInput = $state.raw<{ clearSearch: () => void } | undefined>();
  let removedIds = new Set<number>();
  let trashing = false;
  let renameRequested = $state(0);

  function onRename(model: Model) {
    if (
      !configurationMeta.applicationReadOnly &&
      selected.length === 1 &&
      selected[0].id === model.id
    )
      renameRequested++;
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

  async function fetchNextModelSet() {
    if (busyLoadingNext || loadFailed) return;

    busyLoadingNext = true;
    const generation = loadGeneration;
    try {
      const newModels = await props.modelStream.fetch();
      if (generation === loadGeneration) {
        loadedModels.push(
          ...newModels.filter((model) => !removedIds.has(model.id)),
        );
      }
    } catch (error) {
      if (generation === loadGeneration) {
        loadFailed = true;
        console.error("Failed to load models:", error);
      }
    } finally {
      busyLoadingNext = false;
      if (generation === loadGeneration) initialLoadComplete = true;
    }
  }

  async function resetModelSet() {
    const generation = ++loadGeneration;
    loadedModels = [];
    allModels = [];
    allModelsLoaded = false;
    initialLoadComplete = false;
    while (busyLoadingNext) {
      await wait(50);
    }

    if (generation !== loadGeneration) return;
    loadFailed = false;
    await fetchNextModelSet();
  }

  async function loadAllModels(generation: number) {
    if (loadFailed || generation !== loadGeneration) return;
    try {
      const models = await props.modelStream.getAll();
      if (generation === loadGeneration) {
        allModels = models.filter((model) => !removedIds.has(model.id));
        allModelsLoaded = true;
      }
    } catch (error) {
      if (generation === loadGeneration) {
        loadFailed = true;
        console.error("Failed to load models:", error);
      }
    }
  }

  async function setNewSearchText(newText: string | null) {
    searchText = newText;
    props.modelStream.setSearchText(newText);
    selected = [];
    const reset = resetModelSet();
    const generation = loadGeneration;
    await reset;
    await loadAllModels(generation);
  }

  function clearFilters() {
    fileTypes = [];
    hasFileTypeFilter = false;
    props.modelStream.setFileTypes([]);
    searchInput?.clearSearch();
  }

  $effect(() => {
    props.modelStream.setOrderBy(
      convertOrderOptionModelsToEnum(configuration.order_option_models),
    );
  });

  function onDeleteSelected(models: Model[]) {
    onDelete(models);
  }

  function onGroupDeleteSelected(models: Model[]) {
    if (!props.onRemoveGroupDelete) {
      return;
    }

    onDelete(models.filter((m) => !!m.group));
  }

  function onModelsMoved(models: Model[], destination: GroupMeta) {
    const ids = new Set(models.map((model) => model.id));
    for (const model of [...loadedModels, ...allModels, ...selected]) {
      if (ids.has(model.id)) model.group = destination;
    }
    if (props.onRemoveGroupDelete)
      onDelete(
        models.filter(
          (model) => !!model.group && model.group.id !== destination.id,
        ),
      );
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
      for (const model of [...loadedModels, ...allModels, ...selected]) {
        if (
          ids.has(model.id) &&
          !model.labels.some((item) => item.id === label.id)
        )
          model.labels.push(label);
      }
    };
    window.addEventListener("models-labelled", labelled);
    return () => {
      window.removeEventListener("models-moved", moved);
      window.removeEventListener("models-labelled", labelled);
    };
  });

  function onDelete(models: Model[]) {
    const ids = new Set(models.map((model) => model.id));
    props.modelStream.removeModels?.(models);
    ids.forEach((id) => removedIds.add(id));
    loadedModels = loadedModels.filter((model) => !ids.has(model.id));
    allModels = allModels.filter((model) => !ids.has(model.id));
    selected = selected.filter((model) => !ids.has(model.id));

    if (
      !searchText &&
      !hasFileTypeFilter &&
      allModelsLoaded &&
      loadedModels.length === 0 &&
      allModels.length === 0
    ) {
      props.onEmpty?.();
    }
  }

  let modelStreamLoadGen = 0;

  $effect(() => {
    void props.modelStream;
    removedIds = new Set();

    const gen = ++modelStreamLoadGen;
    untrack(async () => {
      const reset = resetModelSet();
      const generation = loadGeneration;
      await reset;
      if (gen !== modelStreamLoadGen) {
        return;
      }
      await loadAllModels(generation);
    });
  });
</script>

<div class="flex h-full flex-row">
  {#if showLeftSide}
    <div class="flex flex-1 flex-col gap-1" style="min-width: 0;">
      <div class="flex flex-row justify-center gap-3 px-5 py-3">
        <SearchInput bind:this={searchInput} onsearch={setNewSearchText} />

        <FileTypeFilter
          bind:value={fileTypes}
          onchange={async (x) => {
            props.modelStream.setFileTypes(x);
            hasFileTypeFilter = x.length > 0;
            selected = [];
            allModels = [];
            const reset = resetModelSet();
            const generation = loadGeneration;
            await reset;
            await loadAllModels(generation);
          }}
        />

        <SortFilter
          bind:value={configuration.order_option_models}
          options={MODEL_ORDER_LABELS}
          onchange={async (x) => {
            props.modelStream.setOrderBy(convertOrderOptionModelsToEnum(x));
            const reset = resetModelSet();
            const generation = loadGeneration;
            await reset;
            await loadAllModels(generation);
          }}
        />

        <UiSizeFilter bind:value={configuration.size_option_models} />
      </div>

      {#if selected.length > 0}
        <div
          class="flex flex-wrap justify-between gap-2 px-5 text-xs text-muted-foreground"
        >
          <span aria-live="polite">{selected.length} selected</span>
          {#if !configurationMeta.applicationReadOnly}
            <span>Ctrl/⌘+A select loaded · Delete trash · F2 rename</span>
          {/if}
        </div>
      {/if}
      <ModelGridInner
        bind:value={
          () => selected,
          (value) => {
            renameRequested = 0;
            selected = value;
          }
        }
        itemSize={configuration.size_option_models}
        availableModels={loadedModels}
        endOfListReached={fetchNextModelSet}
        {onTrash}
        {onRename}
        {onModelsMoved}
      />
      {#if loadFailed}
        <button type="button" onclick={() => setNewSearchText(searchText)}>
          Failed to load models. Retry
        </button>
      {:else if !initialLoadComplete && loadedModels.length === 0}
        <div role="status" class="flex items-center justify-center gap-2">
          <Spinner /> Loading models
        </div>
      {:else if loadedModels.length === 0}
        <EmptyState>
          {#if searchText || hasFileTypeFilter}
            No matching models
            <button type="button" class="ml-2 underline" onclick={clearFilters}
              >Clear filters</button
            >
          {:else}
            No models found. <a href={resolve("/import")} class="underline"
              >Import models</a
            >
          {/if}
        </EmptyState>
      {/if}
    </div>
  {/if}

  {#if showRightSide}
    <SelectionPanel onClose={() => (selected = [])}>
      {#if selected.length >= 2}
        <MultiModelEdit
          models={selected}
          onDelete={onDeleteSelected}
          onGroupDelete={onGroupDeleteSelected}
          {onModelsMoved}
        />
      {:else if selected.length === 1}
        <ModelEdit
          initialEditMode={props.initialEditMode}
          model={selected[0]}
          {renameRequested}
          onDelete={onDeleteSelected}
          {onModelsMoved}
          onGroupDelete={onGroupDeleteSelected}
        />
      {:else if loadedModels.length === 1}
        <ModelEdit
          initialEditMode={props.initialEditMode}
          model={loadedModels[0]}
          onDelete={onDeleteSelected}
          {onModelsMoved}
          onGroupDelete={onGroupDeleteSelected}
        />
      {:else if props.default_show_multiselect_all}
        <MultiModelEdit
          models={allModelsWithFallback}
          onDelete={onDeleteSelected}
          onGroupDelete={onGroupDeleteSelected}
          {onModelsMoved}
        />
      {:else}
        <EmptyState>No model selected</EmptyState>
      {/if}
    </SelectionPanel>
  {/if}
</div>
