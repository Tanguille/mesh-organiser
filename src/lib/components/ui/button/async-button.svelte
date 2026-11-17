<script lang="ts">
  import type { Snippet } from "svelte";
  import Root from "./button.svelte";

  const props: {
    children: Snippet;
    class?: string;
    onclick: () => Promise<void>;
  } = $props();
  let active = $state(true);

  async function onClick() {
    active = false;
    try {
      await props.onclick();
    } finally {
      active = true;
    }
  }
</script>

<Root onclick={onClick} class={props.class} disabled={!active}>
  {@render props.children?.()}
</Root>
