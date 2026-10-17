<script lang="ts">
  import { cn } from "#lib/utils.js";
  import type { WithElementRef } from "bits-ui";
  import type { Snippet } from "svelte";
  import type { HTMLAttributes } from "svelte/elements";

  let {
    ref = $bindable(null),
    class: className,
    children,
    child,
    size = "default",
    ...restProps
  }: WithElementRef<HTMLAttributes<HTMLButtonElement>, HTMLButtonElement> & {
    size?: "default" | "lg";
    child?: Snippet<[{ props: Record<string, unknown> }]>;
  } = $props();

  const buttonProps = $derived({
    class: cn(
      "peer/menu-button ring-sidebar-ring hover:bg-sidebar-accent hover:text-sidebar-accent-foreground active:bg-sidebar-accent active:text-sidebar-accent-foreground data-[state=open]:hover:bg-sidebar-accent data-[state=open]:hover:text-sidebar-accent-foreground flex w-full items-center gap-2 overflow-hidden rounded-md p-2 text-left text-sm outline-none transition-[width,height,padding] focus-visible:ring-2 disabled:pointer-events-none disabled:opacity-50 group-has-data-[sidebar=menu-action]/menu-item:pr-8 aria-disabled:pointer-events-none aria-disabled:opacity-50 group-data-[collapsible=icon]:size-8! group-data-[collapsible=icon]:p-2! [&>span:last-child]:truncate [&>svg]:size-4 [&>svg]:shrink-0",
      size === "lg"
        ? "h-12 text-sm group-data-[collapsible=icon]:p-0!"
        : "h-8 text-sm",
      className,
    ),
    "data-sidebar": "menu-button",
    "data-size": size,
    ...restProps,
  });
</script>

{#if child}
  {@render child({ props: buttonProps })}
{:else}
  <button bind:this={ref} {...buttonProps}>
    {@render children?.()}
  </button>
{/if}
