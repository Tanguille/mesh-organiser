<script lang="ts">
  import { Input } from "$lib/components/ui/input";
  import { debounce, normalizeSearchInput } from "$lib/utils";

  const props: { onsearch: (text: string | null) => void } = $props();

  const debouncedSearch = debounce(
    (text: string | null) => props.onsearch(text),
    200,
  );
</script>

<Input
  oninput={(event) => debouncedSearch(normalizeSearchInput(event))}
  class="grow border-primary"
  placeholder="Search"
/>
