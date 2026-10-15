<script lang="ts">
  import { cn } from "#lib/utils.js";
  import type { WithElementRef } from "bits-ui";
  import type { HTMLAttributes } from "svelte/elements";
  import { SIDEBAR_WIDTH, SIDEBAR_WIDTH_ICON } from "./constants.js";
  import { setSidebar } from "./context.svelte.js";

  let {
    ref = $bindable(null),
    open = $bindable(true),
    onOpenChange = () => {},
    class: className,
    style,
    children,
    ...restProps
  }: WithElementRef<HTMLAttributes<HTMLDivElement>> & {
    open?: boolean;
    onOpenChange?: (open: boolean) => void;
  } = $props();

  const sidebar = setSidebar({
    open: () => open,
    setOpen: (value: boolean) => {
      open = value;
      onOpenChange(value);
    },
  });
</script>

<svelte:window onkeydown={sidebar.handleShortcutKeydown} />

<div
  style="--sidebar-width: {SIDEBAR_WIDTH}; --sidebar-width-icon: {SIDEBAR_WIDTH_ICON}; {style}"
  class={cn("group/sidebar-wrapper flex min-h-svh w-full", className)}
  bind:this={ref}
  {...restProps}
>
  {@render children?.()}
</div>
