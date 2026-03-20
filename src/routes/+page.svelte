<script lang="ts">
  import { onMount } from "svelte";
  import { getContainer } from "$lib/api/dependency_injection";
  import { IHostApi } from "$lib/api/shared/host_api";
  import { configuration } from "$lib/configuration.svelte";
  import { goto } from "$app/navigation";
  import { resolve } from "$app/paths";

  let version = $state("");

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

  onMount(async () => {
    let hostApi = getContainer().optional<IHostApi>(IHostApi);
    if (hostApi) {
      version = await hostApi.getVersion();
    }
  });
</script>

<main class="container flex h-full flex-col items-center justify-center gap-2">
  <h1 class="font-bold">Mesh Organiser</h1>
  <p class="mb-5">Version {version}</p>
  <img src="/logo.png" class="logo tauri h-40" alt="Mesh Organiser Logo" />
</main>
