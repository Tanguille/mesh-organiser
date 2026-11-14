<script lang="ts">
  import { page } from "$app/state";
  import Home from "@lucide/svelte/icons/home";
  import List from "@lucide/svelte/icons/list";
  import Printer from "@lucide/svelte/icons/printer";
  import Settings from "@lucide/svelte/icons/settings";
  import Upload from "@lucide/svelte/icons/upload";

  type Tab = "library" | "import" | "slice" | "printed" | "settings";

  const tabs: { id: Tab; label: string; href: string; icon: typeof Home }[] = [
    { id: "library", label: "Library", href: "/", icon: Home },
    { id: "import", label: "Import", href: "/import", icon: Upload },
    { id: "slice", label: "Slice", href: "/slice", icon: Printer },
    { id: "printed", label: "Printed", href: "/printed", icon: List },
    { id: "settings", label: "Settings", href: "/settings", icon: Settings },
  ];

  let currentPath = $derived(page.url.pathname);
  // These routes render before the library APIs are registered, so the tabs would lead nowhere.
  let hidden = $derived(
    ["/login", "/panic", "/mobile-server"].some((route) =>
      currentPath.startsWith(route),
    ),
  );

  function isActive(tab: Tab): boolean {
    if (tab === "library") {
      return currentPath === "/" || currentPath.startsWith("/model");
    }
    return currentPath.startsWith(`/${tab}`);
  }
</script>

{#if !hidden}
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
{/if}
