<script lang="ts">
  import {
    Card,
    CardHeader,
    CardTitle,
    CardContent,
  } from "$lib/components/ui/card";
  import { platform } from "@tauri-apps/plugin-os";
  import {
    Button,
    AsyncButton,
    buttonVariants,
  } from "$lib/components/ui/button/index.js";
  import { Separator } from "$lib/components/ui/separator/index.js";
  import Link from "@lucide/svelte/icons/link";

  interface Function {
    (): void;
  }

  import { type Update } from "@tauri-apps/plugin-updater";
  import { toast } from "svelte-sonner";
  import { relaunch } from "@tauri-apps/plugin-process";
  import { configuration } from "$lib/configuration.svelte";

  let props: { update: Update; onDismiss?: Function } = $props();
  let currentPlatform = platform();
  let automaticUpdatePlatforms = ["windows"];

  async function executeUpdate() {
    const update = props.update;

    await update.downloadAndInstall((event) => {
      switch (event.event) {
        case "Started":
          toast.info("Downloading update...");
          break;
      }
    });

    await relaunch();
  }

  function ignore(version: string) {
    configuration.ignore_update = version;
    props.onDismiss!();
  }
</script>

<div
  class="fixed z-50 flex h-full w-full items-center justify-center bg-black/50"
>
  <Card>
    <CardHeader>
      <CardTitle>Update available</CardTitle>
    </CardHeader>
    <CardContent class="flex flex-col gap-4">
      <p>
        A new version of Mesh Organiser (v{props.update.currentVersion} -&gt; v{props
          .update.version}) is available. Do you want to update?
      </p>

      <a
        href="https://github.com/tanguille/mesh-organiser/releases"
        target="_blank"
        class="{buttonVariants({ variant: 'ghost' })} flex w-full gap-2"
      >
        <Link />
        View changelog
      </a>

      <Separator />

      <div class="flex w-full justify-end gap-4">
        <Button variant="outline" onclick={() => props.onDismiss!()}
          >Dismiss</Button
        >
        <Button variant="outline" onclick={() => ignore(props.update.version)}
          >Ignore this update</Button
        >
        <Button variant="destructive" onclick={() => ignore("always")}
          >Never notify again</Button
        >
        {#if automaticUpdatePlatforms.includes(currentPlatform)}
          <AsyncButton onclick={executeUpdate}>Update now</AsyncButton>
        {:else}
          <Button disabled={true}
            >Automatic updates unavailable on {currentPlatform}</Button
          >
        {/if}
      </div>
    </CardContent>
  </Card>
</div>
