<script lang="ts">
  import * as Select from "$lib/components/ui/select/index.js";
  import {
    SizeOptionModelsAsList,
    type SizeOptionModels,
  } from "$lib/api/shared/settings_api";
  import Grid2x2 from "@lucide/svelte/icons/grid-2x2";
  import List from "@lucide/svelte/icons/list";

  let { value = $bindable() }: { value: SizeOptionModels } = $props();
</script>

<Select.Root type="single" name="Size" bind:value>
  <Select.Trigger
    class="w-auto border-primary"
    hideArrow={true}
    title="Display: {value.replaceAll('_', ' ')}"
  >
    {#if value.startsWith("Grid")}
      <Grid2x2 />
    {:else}
      <List />
    {/if}
  </Select.Trigger>
  <Select.Content>
    <Select.Group>
      <Select.GroupHeading>Model display</Select.GroupHeading>
      {#each SizeOptionModelsAsList as entry (entry)}
        <Select.Item value={entry} label={entry.replaceAll("_", " ")}
          >{entry.replaceAll("_", " ")}</Select.Item
        >
      {/each}
    </Select.Group>
  </Select.Content>
</Select.Root>
