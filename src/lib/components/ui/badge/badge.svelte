<script lang="ts">
  import type { WithElementRef } from "bits-ui";
  import type { HTMLAttributes } from "svelte/elements";
  import { cn } from "#lib/utils.js";

  let {
    ref = $bindable(null),
    class: className,
    variant = "default",
    children,
    ...restProps
  }: WithElementRef<HTMLAttributes<HTMLSpanElement>> & {
    variant?: "default" | "outline";
  } = $props();
</script>

<span
  bind:this={ref}
  class={cn(
    "inline-flex items-center rounded-full border px-2.5 py-0.5 text-xs font-semibold transition-colors focus:ring-2 focus:ring-ring focus:ring-offset-2 focus:outline-none",
    variant === "default"
      ? "border-transparent bg-primary text-primary-foreground hover:bg-primary/80"
      : "text-foreground",
    className,
  )}
  {...restProps}
>
  {@render children?.()}
</span>
