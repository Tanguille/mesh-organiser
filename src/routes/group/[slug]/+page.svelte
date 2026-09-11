<script lang="ts">
  import { goto } from "$app/navigation";
  import { resolve } from "$app/paths";
  import { page } from "$app/state";
  import GroupPage from "$lib/components/view/group-page.svelte";

  let group = $derived({ id: parseInt(page.params.slug!) });
</script>

<GroupPage {group} onAllModelsDelete={() => goto(resolve("/group"))} />
