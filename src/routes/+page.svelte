<script lang="ts">
  import { configuration } from "$lib/configuration.svelte";
  import { goto } from "$app/navigation";
  import { resolve } from "$app/paths";
  import AppHeader from "$lib/components/view/app-header.svelte";

  const startupRoutes = {
    models: "/model",
    import: "/import",
    groups: "/group",
    favorites: "/favorite",
    "print-history": "/printed",
    projects: "/resource",
  } as const;

  if (configuration.startup_page) {
    goto(resolve(startupRoutes[configuration.startup_page]));
  }
</script>

<main class="container flex h-full flex-col items-center justify-center gap-2">
  <AppHeader />
</main>
