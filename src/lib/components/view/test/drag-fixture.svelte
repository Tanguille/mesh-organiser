<script lang="ts">
  import type { Model } from "#lib/api/shared/model_api";
  import DragSelectedModelsRoot from "../drag-selected-models-root.svelte";
  import DragSelectedModels from "../drag-selected-models.svelte";
  let { models, onActivate }: { models: Model[]; onActivate?: () => void } =
    $props();
</script>

<DragSelectedModelsRoot>
  <DragSelectedModels {models}>
    <div
      data-testid="source"
      role="option"
      aria-selected="true"
      tabindex="0"
      onclick={onActivate}
      onkeydown={() => {}}
    >
      Source model
    </div>
    <div data-testid="group" data-drag-type="group" data-drag-param="7">
      Group
    </div>
  </DragSelectedModels>
  <div data-drag-type="label" data-drag-param="9">
    <svg aria-label="label icon"><path data-testid="label-path" d="M0 0" /></svg
    >
  </div>
</DragSelectedModelsRoot>
