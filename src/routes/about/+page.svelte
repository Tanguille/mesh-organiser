<script lang="ts">
  import Link from "@lucide/svelte/icons/link";
  import Heart from "@lucide/svelte/icons/heart";
  import { onMount } from "svelte";
  import { getContainer } from "$lib/api/dependency_injection";
  import { IHostApi } from "$lib/api/shared/host_api";

  let version = $state("");

  let repositoryLinks = [
    {
      name: "Project Repository",
      url: "https://github.com/tanguille/mesh-organiser",
    },
    {
      name: "Mesh Thumbnail Generator",
      url: "https://github.com/suchmememanyskill/mesh-thumbnail",
    },
    {
      name: "Report an issue / Request a feature",
      url: "https://github.com/tanguille/mesh-organiser/issues",
    },
  ];

  onMount(async () => {
    let hostApi = getContainer().optional<IHostApi>(IHostApi);
    if (hostApi) {
      version = await hostApi.getVersion();
    }
  });
</script>

<div class="container flex h-full flex-col items-center justify-center gap-2">
  <h1 class="font-bold">Mesh Organiser</h1>
  <p class="mb-5">Version {version}</p>
  <img src="/logo.png" class="logo tauri h-40" alt="Mesh Organiser Logo" />
  <div class="h-10"></div>
  <h1 class="font-bold">Credits</h1>
  <p>
    Originally developed by Sims. Forked by <a
      class="text-primary hover:underline"
      href="https://github.com/tanguille/"
      target="_blank">Tanguille</a
    >
  </p>
  <div class="h-10"></div>
  <h1 class="font-bold">Code/Repositories</h1>
  <p class="mb-2">Contributions are welcome!</p>
  {#each repositoryLinks as link (link.url)}
    <p class="flex items-center gap-1">
      <Link size="16" />
      <a
        href={link.url}
        rel="external"
        target="_blank"
        class="text-primary hover:underline"
      >
        {link.name}
      </a>
    </p>
  {/each}
  <p class="flex items-center gap-1">
    <Heart size="16" /> Written in Tauri and Svelte
  </p>
</div>
