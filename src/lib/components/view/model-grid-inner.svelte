<script lang="ts">
  import { onDestroy } from "svelte";
  import type { ClassValue } from "svelte/elements";
  import RightClickModels from "#lib/components/view/right-click-models.svelte";
  import ModelTiny from "#lib/components/view/model-tiny.svelte";
  import ModelTinyList from "#lib/components/view/model-tiny-list.svelte";
  import Checkbox from "#lib/components/ui/checkbox/checkbox.svelte";
  import DragSelectedModels from "./drag-selected-models.svelte";
  import type { Model } from "#lib/api/shared/model_api";
  import type { GroupMeta } from "#lib/api/shared/group_api";
  import type { SizeOptionModels } from "#lib/api/shared/settings_api";
  import { SizeOptionClasses } from "#lib/components/view/size-classes";
  import { createGridSelection } from "#lib/components/view/grid-selection.svelte";
  import { configuration } from "#lib/configuration.svelte";

  let {
    value = $bindable(),
    itemSize,
    availableModels,
    clazz = undefined,
    endOfListReached = undefined,
    onTrash = undefined,
    onRename = undefined,
    onModelsMoved = undefined,
  }: {
    value: Model[];
    itemSize: SizeOptionModels;
    availableModels: Model[];
    clazz?: ClassValue;
    endOfListReached?: () => void;
    onTrash?: (models: Model[]) => void;
    onRename?: (model: Model) => void;
    onModelsMoved?: (models: Model[], group: GroupMeta) => void;
  } = $props();

  let scrollContainer: HTMLElement;

  const selection = createGridSelection<Model>({
    getItems: () => availableModels,
    getSelected: () => value,
    setSelected: (next) => (value = next),
    getId: (model) => model.id,
    getScrollContainer: () => scrollContainer,
    onEndOfList: () => endOfListReached?.(),
    onRename: (model) => onRename?.(model),
  });

  onDestroy(() => {
    selection.destroy();
  });

  $effect(() => {
    void availableModels;
    selection.reconcileFocus();
  });

  const sizeClasses = $derived(SizeOptionClasses[itemSize]);

  function onKeyDown(model: Model, event: KeyboardEvent) {
    if (
      event.key === "Delete" &&
      !event.repeat &&
      !event.ctrlKey &&
      !event.metaKey &&
      !event.altKey &&
      onTrash &&
      value.length > 0
    ) {
      event.preventDefault();
      event.stopPropagation();
      onTrash(value);
      return;
    }
    selection.onKeyDown(model, event);
  }
</script>

<div
  class="overflow-y-scroll {clazz}"
  role="listbox"
  aria-label="Models"
  aria-multiselectable="true"
  bind:this={scrollContainer}
  onscroll={selection.handleScroll}
  onfocusout={selection.onFocusOut}
>
  <DragSelectedModels models={value} class="select-none">
    <RightClickModels
      models={value}
      {onTrash}
      {onModelsMoved}
      class={`flex flex-row flex-wrap content-start justify-center gap-2 outline-0 ${configuration.show_multiselect_checkboxes && itemSize.includes("Grid") ? "pt-[5px]" : ""}`}
    >
      {#if itemSize.includes("List")}
        {#each availableModels as model (model.id)}
          {@const isSelected = selection.selectedSet.has(model.id)}
          <div class="flex w-full items-center gap-2">
            {@render ModelCheckbox(model, "", isSelected)}
            <div
              role="option"
              tabindex={selection.tabIndex(model)}
              aria-selected={isSelected}
              oncontextmenu={(e) => selection.onRightClick(model, e)}
              onclick={(e) => selection.onClick(model, e)}
              onkeydown={(e) => onKeyDown(model, e)}
              onfocus={() => selection.onFocus(model)}
              onmousedown={(e) => selection.earlyOnClick(model, e, isSelected)}
              class="min-w-0 flex-1 cursor-pointer"
            >
              <ModelTinyList
                {model}
                class="{sizeClasses} pointer-events-none select-none {isSelected
                  ? 'border-primary'
                  : ''}"
              />
            </div>
          </div>
        {/each}
      {:else}
        {#each availableModels as model (model.id)}
          {@const isSelected = selection.selectedSet.has(model.id)}
          <div class="group relative">
            <div
              role="option"
              tabindex={selection.tabIndex(model)}
              aria-selected={isSelected}
              oncontextmenu={(e) => selection.onRightClick(model, e)}
              onclick={(e) => selection.onClick(model, e)}
              onkeydown={(e) => onKeyDown(model, e)}
              onfocus={() => selection.onFocus(model)}
              onmousedown={(e) => selection.earlyOnClick(model, e, isSelected)}
              class="cursor-pointer"
            >
              <ModelTiny
                {model}
                class="{sizeClasses} pointer-events-none select-none {isSelected
                  ? 'border-primary'
                  : ''}"
              />
            </div>
            {@render ModelCheckbox(
              model,
              `absolute top-[-5px] left-[-5px] bg-card rounded-lg ${isSelected ? "" : "group-hover:opacity-100 opacity-0"}`,
              isSelected,
            )}
          </div>
        {/each}
      {/if}
    </RightClickModels>
  </DragSelectedModels>
</div>

{#snippet ModelCheckbox(model: Model, clazz: ClassValue, isSelected: boolean)}
  {#if configuration.show_multiselect_checkboxes}
    <Checkbox
      class={clazz}
      aria-label="Select {model.name}"
      tabindex={-1}
      bind:checked={() => isSelected, (val) => selection.toggle(model, val)}
    />
  {/if}
{/snippet}
