<script lang="ts">
  import { Toggle as TogglePrimitive } from "bits-ui";
  import { cn } from "#lib/utils.js";

  let {
    ref = $bindable(null),
    pressed = $bindable(false),
    class: className,
    ...restProps
  }: TogglePrimitive.RootProps = $props();
</script>

<TogglePrimitive.Root
  bind:ref
  bind:pressed
  class={cn(
    "inline-flex h-9 min-w-9 items-center justify-center gap-2 rounded-md bg-transparent px-2.5 text-sm font-medium ring-offset-background transition-colors hover:bg-muted hover:text-muted-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:outline-none disabled:pointer-events-none disabled:opacity-50 data-[state=on]:bg-primary data-[state=on]:text-primary-foreground [&_svg]:pointer-events-none [&_svg]:size-4 [&_svg]:shrink-0",
    className,
  )}
  {...restProps}
/>
