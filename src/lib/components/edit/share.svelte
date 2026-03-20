<script lang="ts">
  import {
    Card,
    CardHeader,
    CardTitle,
    CardContent,
  } from "$lib/components/ui/card";

  import { Label } from "$lib/components/ui/label";
  import { Input } from "$lib/components/ui/input";

  import { countWriter, debounce } from "$lib/utils";
  import type { ClassValue } from "svelte/elements";
  import * as DropdownMenu from "$lib/components/ui/dropdown-menu/index.js";
  import Ellipsis from "@lucide/svelte/icons/ellipsis";
  import { buttonVariants } from "../ui/button/button.svelte";
  import { getContainer } from "$lib/api/dependency_injection";
  import { updateSidebarState } from "$lib/sidebar_data.svelte";
  import { untrack } from "svelte";
  import { IShareApi, type Share } from "$lib/api/shared/share_api";
  import { toast } from "svelte-sonner";
  import AsyncButton from "../ui/button/async-button.svelte";
  import Trash2 from "@lucide/svelte/icons/trash-2";

  interface Function {
    (): void;
  }

  const props: { share: Share; class?: ClassValue; onDelete?: Function } =
    $props();
  let link = $state<string>("");

  const shareApi = getContainer().require<IShareApi>(IShareApi);

  const saveShareDebounced = debounce(async (editedShare: Share) => {
    await shareApi.editShare(editedShare);
  }, 1000);

  function onUpdateShare() {
    let snapshot = $state.snapshot(props.share);
    saveShareDebounced(snapshot);
  }

  async function copyToClipboard() {
    await navigator.clipboard.writeText(link);
    toast.success("Share link copied to clipboard");
  }

  async function deleteShare() {
    await shareApi.deleteShare(props.share);
    if (props.onDelete) props.onDelete();
    await updateSidebarState();
  }

  let linkLoadGen = 0;

  // Re-fetch when share identity changes; `void share.id` keeps the effect keyed to id (not every field).
  $effect(() => {
    const share = $state.snapshot(props.share);
    void share.id;
    const gen = ++linkLoadGen;
    untrack(async () => {
      try {
        const l = await shareApi.getShareLink(share);
        if (gen !== linkLoadGen) {
          return;
        }
        link = l;
      } catch (e) {
        if (gen !== linkLoadGen) {
          return;
        }
        link = "";
        toast.error(
          e instanceof Error ? e.message : "Failed to load share link",
        );
      }
    });
  });
</script>

<Card class="w-full {props.class}">
  <CardHeader class="relative">
    <CardTitle class="break-all">Share: {props.share.shareName}</CardTitle>
    <p class="text-sm">
      Shared by user {props.share.userName}. Contains {countWriter(
        "model",
        props.share.modelIds,
      )}
    </p>

    <div class="absolute top-5 right-0 mr-8">
      <DropdownMenu.Root>
        <DropdownMenu.Trigger>
          <Ellipsis />
        </DropdownMenu.Trigger>
        <DropdownMenu.Content side="right" align="start">
          <DropdownMenu.Item onclick={deleteShare}>
            <Trash2 /> Delete share
          </DropdownMenu.Item>
        </DropdownMenu.Content>
      </DropdownMenu.Root>
    </div>
  </CardHeader>
  <CardContent class="flex flex-col gap-4">
    <div class="flex flex-col space-y-1.5">
      <Label for="share_name_{props.share.id}">Share name</Label>
      <Input
        id="share_name_{props.share.id}"
        type="text"
        class="grow"
        oninput={onUpdateShare}
        bind:value={props.share.shareName}
      />
    </div>
    <div class="flex flex-col space-y-1.5">
      <Label>Share link</Label>
      <div class="flex flex-row gap-2">
        <Input type="text" class="grow" readonly={true} value={link} />
        <AsyncButton onclick={copyToClipboard}>Copy</AsyncButton>
        <a
          href={link}
          rel="external"
          target="_blank"
          class={buttonVariants({ variant: "default" })}>Open</a
        >
      </div>
    </div>
  </CardContent>
</Card>
