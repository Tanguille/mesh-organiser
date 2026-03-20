import { toast } from "svelte-sonner";
import { countWriter } from "$lib/utils";
import type { Model } from "./api/shared/model_api";
import { ILabelApi, type LabelMeta } from "./api/shared/label_api";
import { getContainer } from "./api/dependency_injection";
import { sidebarState, updateSidebarState } from "./sidebar_data.svelte";

export const state = $state({
  dragging_models: [] as Model[],
  dragging: false,
});

export function startDragging(models: Model[]) {
  state.dragging_models = models;
  state.dragging = true;
}

export function stopDragging() {
  state.dragging_models = [];
  state.dragging = false;
}

export async function addModelsToLabel(label: LabelMeta) {
  const labelApi = getContainer().require<ILabelApi>(ILabelApi);
  const models = $state.snapshot(state.dragging_models);
  const promise = labelApi.addLabelToModels(label, models);

  toast.promise(promise, {
    loading: `Adding label ${label.name} to ${countWriter("model", models)}...`,
    success: (_) => {
      return `Added label ${label.name} to ${countWriter("model", models)}`;
    },
  });

  await promise;
  await updateSidebarState();
}

export async function addModelsToLabelId(label_id: number) {
  const label = $state.snapshot(
    sidebarState.labels.find((l) => l.meta.id === label_id),
  );

  if (label) {
    await addModelsToLabel(label.meta);
  }
}
