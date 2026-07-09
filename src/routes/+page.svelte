<script lang="ts">
  import { configuration } from "$lib/configuration.svelte";
  import { goto } from "$app/navigation";
  import { resolve } from "$app/paths";
  import AppHeader from "$lib/components/view/app-header.svelte";

  switch (configuration.startup_page) {
    case "models":
      goto(resolve("/model"));
      break;
    case "import":
      goto(resolve("/import"));
      break;
    case "groups":
      goto(resolve("/group"));
      break;
    case "favorites":
      goto(resolve("/favorite"));
      break;
    case "print-history":
      goto(resolve("/printed"));
      break;
    case "projects":
      goto(resolve("/resource"));
      break;
  }
</script>

<main class="container flex h-full flex-col items-center justify-center gap-2">
  <AppHeader />
</main>
