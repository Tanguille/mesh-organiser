<script lang="ts" generics="O extends string">
  import * as Select from "$lib/components/ui/select/index.js";
  import ArrowDownWideNarrow from "@lucide/svelte/icons/arrow-down-wide-narrow";
  import ArrowDownNarrowWide from "@lucide/svelte/icons/arrow-down-narrow-wide";

  // `options` is one of the *_ORDER_LABELS maps from settings_api.
  let {
    value = $bindable(),
    options,
    onchange = () => {},
  }: {
    value: O;
    options: Record<O, string>;
    onchange?: (order: O) => void;
  } = $props();
</script>

<Select.Root
  type="single"
  name="Sort"
  bind:value
  onValueChange={(x) => onchange(x as O)}
>
  <Select.Trigger
    class="w-auto border-primary"
    hideArrow={true}
    title="Sort on: {options[value]}"
  >
    {#if value.endsWith("asc")}
      <ArrowDownNarrowWide />
    {:else}
      <ArrowDownWideNarrow />
    {/if}
  </Select.Trigger>
  <Select.Content>
    <Select.Group>
      <Select.GroupHeading>Sort on</Select.GroupHeading>
      {#each Object.entries(options) as [key, label] (key)}
        <Select.Item value={key} label={label as string}>{label}</Select.Item>
      {/each}
    </Select.Group>
  </Select.Content>
</Select.Root>
