<script lang="ts">
  import { panicState } from "#lib/configuration.svelte";
  import Bomb from "@lucide/svelte/icons/bomb";
  import { Button } from "#lib/components/ui/button/index.js";
</script>

<div class="flex h-full w-full flex-col items-center justify-center px-4">
  <div class="mb-4 rounded-md bg-primary p-2">
    <Bomb />
  </div>
  <h1 class="mb-2 text-2xl font-bold">Application Error</h1>
  <p class="mb-4 text-center whitespace-pre">{panicState.message}</p>
  {#if panicState.serverSetup}
    <Button href="/mobile-server" variant="default">Configure server</Button>
  {/if}
</div>
