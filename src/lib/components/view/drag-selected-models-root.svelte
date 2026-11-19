<script lang="ts">
  import type { Snippet } from "svelte";
  import type { ClassValue } from "svelte/elements";
  import {
    addModelsToLabelId,
    state as dragState,
    stopDragging,
  } from "#lib/drag-selected-models.svelte";
  import Boxes from "@lucide/svelte/icons/boxes";
  import Badge from "#lib/components/ui/badge/badge.svelte";
  import { getContainer } from "#lib/api/dependency_injection";
  import { getGroupById, IGroupApi } from "#lib/api/shared/group_api";
  import { updateSidebarState } from "#lib/sidebar_data.svelte";
  import { configurationMeta } from "#lib/configuration.svelte";
  import { toast } from "svelte-sonner";
  import { onDestroy } from "svelte";

  const props: { children: Snippet; class?: ClassValue } = $props();

  let currentX = $state.raw(0);
  let currentY = $state.raw(0);
  let highlighted: Element | null = null;
  let dropIntent = $state<string | null>(null);

  function highlight(target: EventTarget | null) {
    highlighted?.classList.remove("ring-2", "ring-primary");
    highlighted = null;
    dropIntent = null;
    const candidate =
      target instanceof Element
        ? target.closest("[data-drag-type][data-drag-param]")
        : null;
    if (!candidate || configurationMeta.applicationReadOnly) return;
    const type = candidate.getAttribute("data-drag-type");
    const id = Number(candidate.getAttribute("data-drag-param"));
    if (!Number.isInteger(id) || id < 0) return;
    if (
      type === "group" &&
      dragState.dragging_models.some((model) => model.group?.id !== id)
    ) {
      dropIntent = "Move to group";
    } else if (
      type === "label" &&
      dragState.dragging_models.some(
        (model) => !model.labels.some((label) => label.id === id),
      )
    ) {
      dropIntent = "Add label";
    }
    if (!dropIntent) return;
    highlighted = candidate;
    highlighted?.classList.add("ring-2", "ring-primary");
  }

  function onmousemove(event: MouseEvent) {
    if (dragState.dragging) {
      currentX = event.clientX;
      currentY = event.clientY;
      highlight(event.target);
    }
  }

  function onmouseup(event: MouseEvent) {
    if (!dragState.dragging) {
      return;
    }

    const target =
      event.target instanceof Element
        ? event.target.closest("[data-drag-type][data-drag-param]")
        : null;
    const type = target?.getAttribute("data-drag-type");
    const id = Number(target?.getAttribute("data-drag-param"));
    const models = $state.snapshot(dragState.dragging_models);
    if (
      !configurationMeta.applicationReadOnly &&
      target &&
      Number.isInteger(id)
    ) {
      if (type === "label") {
        void addModelsToLabelId(id).catch((error) => {
          console.error("Failed to add label:", error);
          toast.error("Could not add label");
        });
      } else if (
        type === "group" &&
        id >= 0 &&
        models.some((model) => model.group?.id !== id)
      ) {
        void moveToGroup(id, models);
      }
    }
    highlight(null);
    stopDragging();
  }

  async function moveToGroup(
    id: number,
    models: typeof dragState.dragging_models,
  ) {
    try {
      const groupApi = getContainer().require<IGroupApi>(IGroupApi);
      const group = await getGroupById(groupApi, id);
      if (!group) throw new Error("Group not found");
      await groupApi.addModelsToGroup(group.meta, models);
      window.dispatchEvent(
        new CustomEvent("models-moved", {
          detail: { models, group: group.meta },
        }),
      );
      toast.success(`Moved models to '${group.meta.name}'`);
      await updateSidebarState().catch((error) =>
        console.error("Failed to refresh sidebar:", error),
      );
    } catch (error) {
      console.error("Failed to move models:", error);
      toast.error("Could not move models to group");
    }
  }

  function cancelDrag() {
    highlight(null);
    stopDragging();
  }

  onDestroy(cancelDrag);
</script>

<svelte:window
  {onmousemove}
  {onmouseup}
  onblur={cancelDrag}
  onmouseout={(event) => {
    if (dragState.dragging && !event.relatedTarget) cancelDrag();
  }}
  onkeydown={(event) => {
    if (event.key === "Escape" && dragState.dragging) {
      event.stopImmediatePropagation();
      cancelDrag();
    }
  }}
/>

<div class={props.class}>
  {#if dragState.dragging}
    <div
      class="pointer-events-none fixed z-50 rounded-lg border-2 border-primary p-2 opacity-95"
      style="top: {currentY + 5}px; left: {currentX + 5}px;"
    >
      <Boxes class="h-12 w-12" />
      <div
        class="absolute top-0 left-0 flex h-full w-full items-center justify-center"
      >
        <Badge>{dragState.dragging_models.length}</Badge>
      </div>
      {#if dropIntent}<span>{dropIntent}</span>{/if}
    </div>
  {/if}

  {@render props.children?.()}
</div>
