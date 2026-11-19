<script lang="ts">
  import type { IModelStreamManager } from "#lib/api/shared/model_api";
  import DragSelectedModelsRoot from "../drag-selected-models-root.svelte";
  import ModelGrid from "../model-grid.svelte";
  let { modelStream }: { modelStream: IModelStreamManager } = $props();
</script>

<DragSelectedModelsRoot>
  <ModelGrid {modelStream} />
  <div data-testid="group-target" data-drag-type="group" data-drag-param="7">
    Group destination
  </div>
  <div data-testid="label-target" data-drag-type="label" data-drag-param="9">
    Label destination
  </div>
</DragSelectedModelsRoot>
