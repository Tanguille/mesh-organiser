<script lang="ts">
  import type { Group } from "#lib/api/shared/group_api";
  const { group }: { group: Group } = $props();
</script>

<h2>{group.meta.name}</h2>
<span data-testid="aggregate-{group.meta.id}">
  {JSON.stringify({
    flags: group.flags,
    labels: group.labels.map((label) => label.name).sort(),
  })}
</span>
