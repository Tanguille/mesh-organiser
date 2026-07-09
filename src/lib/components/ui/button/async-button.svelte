<script lang="ts">
  import type { Snippet } from "svelte";
  import Root from "./button.svelte";

  const props: {
    children: Snippet;
    class?: string;
    onclick: () => Promise<void>;
    enabled?: boolean;
  } = $props();
  let active = $state(true);

  async function onClick() {
    active = false;
    await props.onclick();
    active = true;
  }
</script>

<Root
  onclick={onClick}
  class={props.class}
  disabled={!(props.enabled ?? true) || !active}
>
  {@render props.children?.()}
</Root>
