<script lang="ts">
  import { page } from "$app/state";
  import type { IconProps } from "@lucide/svelte";
  import type { Component } from "svelte";
  import Home from "@lucide/svelte/icons/home";
  import List from "@lucide/svelte/icons/list";
  import Printer from "@lucide/svelte/icons/printer";
  import Upload from "@lucide/svelte/icons/upload";

  type Tab = "library" | "import" | "slice" | "print";

  type TabIcon = Component<IconProps, Record<string, unknown>, "">;

  const tabs: { id: Tab; label: string; href: string; icon: TabIcon }[] = [
    { id: "library", label: "Library", href: "/", icon: Home },
    { id: "import", label: "Import", href: "/import", icon: Upload },
    { id: "slice", label: "Slice", href: "/slice", icon: Printer },
    { id: "print", label: "Printed", href: "/print", icon: List },
  ];

  let currentPath = $derived(page.url.pathname);

  function isActive(tab: Tab): boolean {
    if (tab === "library") {
      return currentPath === "/" || currentPath.startsWith("/models");
    }
    return currentPath.startsWith(`/${tab}`);
  }
</script>

<nav
  class="fixed right-0 bottom-0 left-0 z-1000 flex h-[60px] items-center justify-around border-t border-border bg-background px-4"
>
  {#each tabs as tab (tab.id)}
    <a
      href={tab.href}
      class="flex flex-col items-center gap-1 rounded-lg p-2 transition-all duration-200 hover:bg-muted {isActive(
        tab.id,
      )
        ? 'bg-muted text-primary'
        : 'text-muted-foreground'}"
    >
      <tab.icon class="h-5 w-5" />
      <span class="text-xs font-medium">{tab.label}</span>
    </a>
  {/each}
</nav>
