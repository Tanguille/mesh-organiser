<script lang="ts">
  import { Input } from "#lib/components/ui/input";
  import Button from "#lib/components/ui/button/button.svelte";
  import { normalizeSearchInput } from "#lib/utils";
  import X from "@lucide/svelte/icons/x";
  import { onDestroy } from "svelte";

  const props: { onsearch: (text: string | null) => void } = $props();

  let search = $state("");
  let input = $state<HTMLInputElement | null>(null);
  let timeoutId: number | undefined;

  function onInput(event: Event) {
    const text = normalizeSearchInput(event);
    window.clearTimeout(timeoutId);
    timeoutId = window.setTimeout(() => props.onsearch(text), 200);
  }

  export function clearSearch() {
    window.clearTimeout(timeoutId);
    search = "";
    props.onsearch(null);
    input?.focus();
  }

  onDestroy(() => window.clearTimeout(timeoutId));
</script>

<div class="relative flex min-w-0 grow">
  <Input
    bind:ref={input}
    bind:value={search}
    oninput={onInput}
    class="grow border-primary pr-10"
    aria-label="Search"
    placeholder="Search"
  />
  {#if search}
    <Button
      variant="ghost"
      size="icon"
      class="absolute top-0 right-0"
      aria-label="Clear search"
      onclick={clearSearch}><X /></Button
    >
  {/if}
</div>
