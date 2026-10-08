<script lang="ts">
  import type { LucideIcon } from "@lucide/svelte";
  import * as DropdownMenu from "$lib/components/ui/dropdown-menu/index.js";
  import Ellipsis from "@lucide/svelte/icons/ellipsis";

  const {
    label,
    icon: Icon,
    onclick,
  }: { label: string; icon: LucideIcon; onclick: () => void } = $props();
</script>

<DropdownMenu.Root>
  <DropdownMenu.Trigger>
    <Ellipsis />
  </DropdownMenu.Trigger>
  <DropdownMenu.Content side="right" align="start">
    <DropdownMenu.Item {onclick}>
      <Icon />
      {label}
    </DropdownMenu.Item>
  </DropdownMenu.Content>
</DropdownMenu.Root>
