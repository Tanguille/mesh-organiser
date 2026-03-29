<script lang="ts">
  import { goto } from "$app/navigation";
  import { writable } from "svelte/store";
  import { Home, Upload, Printer, List } from "@lucide/svelte";

  // Current tab store
  const currentTab = writable<"library" | "import" | "slice" | "print">(
    "library",
  );

  // Tab change functions
  function goToLibrary() {
    currentTab.set("library");
    goto("/");
  }

  function goToImport() {
    currentTab.set("import");
    goto("/import");
  }

  function goToSlice() {
    currentTab.set("slice");
    goto("/slice");
  }

  function goToPrint() {
    currentTab.set("print");
    goto("/print");
  }

  // Derived for template use
  let tab = $currentTab;
</script>

<div
  class="fixed right-0 bottom-0 left-0 z-[1000] flex h-[60px] items-center justify-around border-t border-border bg-background px-4"
>
  <button
    class="flex flex-col items-center gap-1 rounded-lg p-2 transition-all duration-200 hover:bg-muted {tab ===
    'library'
      ? 'bg-muted text-primary'
      : ''}"
    onclick={goToLibrary}
  >
    <Home class="h-5 w-5" />
    <span class="text-xs font-medium">Library</span>
  </button>

  <button
    class="flex flex-col items-center gap-1 rounded-lg p-2 transition-all duration-200 hover:bg-muted {tab ===
    'import'
      ? 'bg-muted text-primary'
      : ''}"
    onclick={goToImport}
  >
    <Upload class="h-5 w-5" />
    <span class="text-xs font-medium">Import</span>
  </button>

  <button
    class="flex flex-col items-center gap-1 rounded-lg p-2 transition-all duration-200 hover:bg-muted {tab ===
    'slice'
      ? 'bg-muted text-primary'
      : ''}"
    onclick={goToSlice}
  >
    <Printer class="h-5 w-5" />
    <span class="text-xs font-medium">Slice</span>
  </button>

  <button
    class="flex flex-col items-center gap-1 rounded-lg p-2 transition-all duration-200 hover:bg-muted {tab ===
    'print'
      ? 'bg-muted text-primary'
      : ''}"
    onclick={goToPrint}
  >
    <List class="h-5 w-5" />
    <span class="text-xs font-medium">Print</span>
  </button>
</div>
