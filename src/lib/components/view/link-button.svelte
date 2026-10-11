<script lang="ts">
  import { getContainer } from "$lib/api/dependency_injection";
  import { IInternalBrowserApi } from "$lib/api/shared/internal_browser_api";
  import {
    buttonVariants,
    Button,
    type ButtonVariant,
  } from "$lib/components/ui/button/index.js";
  import { configuration } from "$lib/configuration.svelte";
  import Link from "@lucide/svelte/icons/link";
  import type { ClassValue } from "svelte/elements";

  const props: {
    link: string | undefined | null;
    visible?: boolean;
    class?: ClassValue;
    variant?: ButtonVariant;
    withText?: boolean;
    withFallback?: boolean;
  } = $props();
  const variant = $derived(props.variant ?? "default");
  let internalBrowserApi =
    getContainer().optional<IInternalBrowserApi>(IInternalBrowserApi);

  async function openLink() {
    if (props.link && internalBrowserApi) {
      await internalBrowserApi.openInternalBrowser(props.link);
    }
  }
</script>

{#snippet body()}
  <Link />
  {#if props.withText ?? true}
    Open Link
  {/if}
{/snippet}

{#if props.visible ?? !!props.link}
  {#if configuration.open_links_in_external_browser || !internalBrowserApi}
    <a
      href={props.link}
      rel="external"
      target="_blank"
      class="{buttonVariants({ variant })} {props.class}"
    >
      {@render body()}
    </a>
  {:else}
    <Button {variant} class={props.class} onclick={openLink}>
      {@render body()}
    </Button>
  {/if}
{:else if props.withFallback}
  <Button {variant} class={props.class} disabled>
    {@render body()}
  </Button>
{/if}
