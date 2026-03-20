<script lang="ts">
  import * as Select from "$lib/components/ui/select/index.js";
  import type { ClassValue } from "svelte/elements";
  import {
    type OrderOptionGroups,
    type OrderOptionModels,
  } from "$lib/api/shared/settings_api";
  import ArrowDownWideNarrow from "@lucide/svelte/icons/arrow-down-wide-narrow";
  import ArrowDownNarrowWide from "@lucide/svelte/icons/arrow-down-narrow-wide";

  interface OnChangeCallback<T> {
    (newOrderOption: T): void;
  }

  type Props =
    | {
        value: OrderOptionModels;
        subset: "models";
        class?: ClassValue;
        onchange?: OnChangeCallback<OrderOptionModels>;
      }
    | {
        value: OrderOptionGroups;
        subset: "groups";
        class?: ClassValue;
        onchange?: OnChangeCallback<OrderOptionGroups>;
      };

  let {
    value = $bindable(),
    onchange = () => {},
    ...restProps
  }: Props = $props();

  const readableOrders = {
    "date-asc": "Added (Asc)",
    "date-desc": "Added (Desc)",
    "name-asc": "Name (A->Z)",
    "name-desc": "Name (Z->A)",
    "size-asc": "Size (Asc)",
    "size-desc": "Size (Desc)",
    "modified-asc": "Modified (Asc)",
    "modified-desc": "Modified (Desc)",
  };

  const filteredOrders: { [key: string]: string } = $derived.by(() => {
    if (restProps.subset === "groups") {
      let localOrders = { ...readableOrders } as { [key: string]: string };
      delete localOrders["size-asc"];
      delete localOrders["size-desc"];
      return localOrders;
    }

    return readableOrders;
  });
</script>

<Select.Root
  type="single"
  name="Sort"
  onValueChange={() =>
    restProps.subset === "groups"
      ? (onchange as OnChangeCallback<OrderOptionGroups>)(
          $state.snapshot(value) as OrderOptionGroups,
        )
      : (onchange as OnChangeCallback<OrderOptionModels>)(
          $state.snapshot(value) as OrderOptionModels,
        )}
  bind:value
>
  <Select.Trigger
    class="w-auto border-primary {restProps.class}"
    hideArrow={true}
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
      {#each Object.entries(filteredOrders) as order (order[0])}
        <Select.Item value={order[0]} label={order[1]}>{order[1]}</Select.Item>
      {/each}
    </Select.Group>
  </Select.Content>
</Select.Root>
