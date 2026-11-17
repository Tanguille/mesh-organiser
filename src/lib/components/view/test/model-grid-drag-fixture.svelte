<script lang="ts">
  import type { IModelStreamManager } from "#lib/api/shared/model_api";
  import DragSelectedModelsRoot from "../drag-selected-models-root.svelte";
  import ModelGrid from "../model-grid.svelte";
  let { modelStream }: { modelStream: IModelStreamManager } = $props();
</script>

<DragSelectedModelsRoot>
  <ModelGrid {modelStream} />
</DragSelectedModelsRoot>
