<script lang="ts">
  import type { Snippet } from "svelte";
  import Root from "./button.svelte";

  interface Function {
    (): Promise<void>;
  }

  const props: {
    children: Snippet;
    class?: string;
    onclick: Function;
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
