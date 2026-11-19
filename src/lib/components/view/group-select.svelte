<script lang="ts">
  import { getContainer } from "#lib/api/dependency_injection";
  import {
    getAllGroups,
    IGroupApi,
    type GroupMeta,
  } from "#lib/api/shared/group_api";
  import type { Model } from "#lib/api/shared/model_api";
  import * as Popover from "#lib/components/ui/popover/index.js";
  import { Input } from "#lib/components/ui/input/index.js";
  import Button, {
    buttonVariants,
  } from "#lib/components/ui/button/button.svelte";
  import { updateSidebarState } from "#lib/sidebar_data.svelte";
  import { toast } from "svelte-sonner";

  const props: {
    models: Model[];
    onModelsMoved?: (models: Model[], group: GroupMeta) => void;
    menuItem?: boolean;
  } = $props();

  const groupApi = getContainer().require<IGroupApi>(IGroupApi);
  let open = $state(false);
  let groups = $state<GroupMeta[]>([]);
  let search = $state("");
  let loading = $state(false);
  let moving = $state(false);
  let error = $state<string | null>(null);
  const matches = $derived(
    groups.filter((group) =>
      group.name.toLowerCase().includes(search.toLowerCase()),
    ),
  );

  async function loadGroups() {
    loading = true;
    error = null;
    try {
      groups = (await getAllGroups(groupApi))
        .map((group) => group.meta)
        .filter((group) => group.id >= 0);
    } catch (cause) {
      console.error("Failed to load groups:", cause);
      error = "Could not load groups.";
    } finally {
      loading = false;
    }
  }

  async function move(group: GroupMeta) {
    if (moving || props.models.every((model) => model.group?.id === group.id))
      return;
    const models = [...props.models];
    const affected = $state.snapshot(models);
    moving = true;
    error = null;
    try {
      await groupApi.addModelsToGroup(group, affected);
      models.forEach((model) => (model.group = group));
      props.onModelsMoved?.(affected, group);
      open = false;
      toast.success(
        `Moved ${models.length === 1 ? "model" : "models"} to '${group.name}'`,
      );
      updateSidebarState().catch((cause) =>
        console.error("Failed to refresh sidebar:", cause),
      );
    } catch (cause) {
      console.error("Failed to move models:", cause);
      error = "Could not move models. Try again.";
    } finally {
      moving = false;
    }
  }
</script>

<Popover.Root
  bind:open
  onOpenChange={(value) => {
    if (value) loadGroups();
  }}
>
  <Popover.Trigger
    role={props.menuItem ? "menuitem" : undefined}
    class={props.menuItem
      ? "relative flex w-full cursor-default items-center rounded-sm py-1.5 pr-2 pl-8 text-left text-sm outline-none hover:bg-accent focus:bg-accent"
      : buttonVariants({ variant: "default" })}
    disabled={moving}
  >
    Move to group
  </Popover.Trigger>
  <Popover.Content
    class="flex w-80 flex-col gap-2"
    portalProps={{ disabled: props.menuItem }}
  >
    <Input
      aria-label="Search groups"
      placeholder="Search groups"
      bind:value={search}
    />
    {#if loading}
      <p>Loading groups…</p>
    {:else if error}
      <p role="alert">{error}</p>
      <Button variant="outline" onclick={loadGroups}>Retry</Button>
    {:else if matches.length === 0}
      <p>No groups found</p>
    {:else}
      <div class="flex max-h-64 flex-col overflow-y-auto">
        {#each matches as group (group.id)}
          <Button
            variant="ghost"
            class="justify-start"
            disabled={moving ||
              props.models.every((model) => model.group?.id === group.id)}
            onclick={() => move(group)}>{group.name}</Button
          >
        {/each}
      </div>
    {/if}
  </Popover.Content>
</Popover.Root>
