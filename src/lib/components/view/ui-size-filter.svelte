<script lang="ts">
  import * as Select from "$lib/components/ui/select/index.js";
  import type { ClassValue } from "svelte/elements";
  import {
    SizeOptionModelsAsList,
    type SizeOptionModels,
  } from "$lib/api/shared/settings_api";
  import Grid2x2 from "@lucide/svelte/icons/grid-2x2";
  import List from "@lucide/svelte/icons/list";

  interface OnChangeCallback<T> {
    (newOrderOption: T): void;
  }

  type Props = {
    value: SizeOptionModels;
    class?: ClassValue;
    onchange?: OnChangeCallback<SizeOptionModels>;
  };

  let {
    value = $bindable(),
    onchange = () => {},
    ...restProps
  }: Props = $props();
</script>

<Select.Root
  type="single"
  name="Size"
  bind:value
  onValueChange={(v) => {
    if (v != null) {
      onchange(v as SizeOptionModels);
    }
  }}
>
  <Select.Trigger
    class="w-auto border-primary {restProps.class}"
    hideArrow={true}
  >
    {#if value?.includes("Grid") ?? false}
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
