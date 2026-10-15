<script lang="ts">
  import * as Sheet from "#lib/components/ui/sheet/index.js";
  import { cn } from "#lib/utils.js";
  import type { WithElementRef } from "bits-ui";
  import type { HTMLAttributes } from "svelte/elements";
  import { SIDEBAR_WIDTH_MOBILE } from "./constants.js";
  import { useSidebar } from "./context.svelte.js";

  let {
    ref = $bindable(null),
    class: className,
    children,
    ...restProps
  }: WithElementRef<HTMLAttributes<HTMLDivElement>> = $props();

  const sidebar = useSidebar();
</script>

{#if sidebar.isMobile}
  <Sheet.Root
    bind:open={() => sidebar.openMobile, (open) => sidebar.setOpenMobile(open)}
    {...restProps}
  >
    <Sheet.Content
      data-sidebar="sidebar"
      data-mobile="true"
      class="w-(--sidebar-width) bg-sidebar p-0 text-sidebar-foreground [&>button]:hidden"
      style="--sidebar-width: {SIDEBAR_WIDTH_MOBILE};"
    >
      <div class="flex h-full w-full flex-col">
        {@render children?.()}
      </div>
    </Sheet.Content>
  </Sheet.Root>
{:else}
  <div
    bind:this={ref}
    class="group peer hidden text-sidebar-foreground md:block"
    data-state={sidebar.state}
    data-collapsible={sidebar.state === "collapsed" ? "icon" : ""}
  >
    <!-- This is what handles the sidebar gap on desktop -->
    <div
      class="relative h-svh w-(--sidebar-width) bg-transparent transition-[width] duration-200 ease-linear group-data-[collapsible=icon]:w-(--sidebar-width-icon)"
    ></div>
    <div
      class={cn(
        "fixed inset-y-0 left-0 z-10 hidden h-svh w-(--sidebar-width) border-r transition-[left,right,width] duration-200 ease-linear group-data-[collapsible=icon]:w-(--sidebar-width-icon) md:flex",
        className,
      )}
      {...restProps}
    >
      <div
        data-sidebar="sidebar"
        class="flex h-full w-full flex-col bg-sidebar"
      >
        {@render children?.()}
      </div>
    </div>
  </div>
{/if}
