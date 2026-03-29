<script lang="ts">
  import * as Card from "$lib/components/ui/card/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { fetch as tauriFetch } from "@tauri-apps/plugin-http";
  import { invoke, isTauri } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { HttpMethod } from "$lib/api/shared/server_request_api";
  import { ServerRequestApi } from "$lib/api/web/request";
  import { normalizeServerBaseUrl } from "$lib/api/mobile/server_url";
  import { setServerUrl } from "$lib/api/tauri/server_url";
  import { goto } from "$app/navigation";
  import { resolve } from "$app/paths";
  import { toast } from "svelte-sonner";
  import Spinner from "$lib/components/view/spinner.svelte";

  type Gate = "loading" | "web" | "desktop" | "mobile";
  let gate = $state<Gate>("loading");
  let serverUrl = $state("");
  let checking = $state(false);
  let saving = $state(false);
  let testHint = $state<string | null>(null);

  onMount(async () => {
    if (!isTauri()) {
      gate = "web";
      return;
    }
    try {
      gate = (await invoke<boolean>("is_mobile")) ? "mobile" : "desktop";
    } catch {
      gate = "desktop";
    }
  });

  function stripBaseInput(raw: string): string {
    return raw.trim().replace(/\/+$/, "");
  }

  async function testConnection() {
    testHint = null;
    const base = stripBaseInput(serverUrl);
    if (base.length === 0) {
      toast.error("Enter a server URL first.");
      return;
    }

    checking = true;
    try {
      const api = new ServerRequestApi(base, tauriFetch, "include");
      await api.request<unknown>("/users/me", HttpMethod.GET);
      testHint = "Connected. You are signed in on this device.";
      toast.success("Server reachable (authenticated).");
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (msg.includes("status 401")) {
        testHint =
          "Server reachable. Sign in after saving the URL (401 is expected when not logged in).";
        toast.success("Server reachable.");
      } else {
        testHint = null;
        toast.error("Connection test failed.", { description: msg });
      }
    } finally {
      checking = false;
    }
  }

  async function saveAndReload() {
    saving = true;
    testHint = null;
    try {
      const normalized = normalizeServerBaseUrl(serverUrl);
      await setServerUrl(normalized);
      await goto(resolve("/"));
      location.reload();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      toast.error("Could not save server URL.", { description: msg });
    } finally {
      saving = false;
    }
  }
</script>

<div class="flex h-full items-center justify-center p-4">
  {#if gate === "loading"}
    <Spinner />
  {:else if gate === "web"}
    <p class="text-muted-foreground text-center text-sm">
      Remote server setup is only available in the Mesh Organiser mobile app.
    </p>
  {:else if gate === "desktop"}
    <p class="text-muted-foreground text-center text-sm">
      Remote server setup is only available on mobile builds.
    </p>
  {:else}
    <Card.Root class="w-full max-w-md">
      <Card.Header>
        <Card.Title>Remote server</Card.Title>
        <Card.Description>
          Enter the base URL of your Mesh Organiser server (same host you use in
          the browser), e.g. https://nas.example.com:3000
        </Card.Description>
      </Card.Header>
      <Card.Content class="flex flex-col gap-4">
        <div class="flex flex-col gap-2">
          <Label for="mobile-server-url">Server URL</Label>
          <Input
            id="mobile-server-url"
            type="url"
            autocomplete="url"
            placeholder="https://example.com:3000"
            bind:value={serverUrl}
          />
        </div>
        {#if testHint}
          <p class="text-muted-foreground text-sm">{testHint}</p>
        {/if}
        <div class="flex flex-wrap gap-2">
          <Button
            type="button"
            variant="secondary"
            disabled={checking || saving}
            onclick={testConnection}
          >
            {checking ? "Testing…" : "Test"}
          </Button>
          <Button
            type="button"
            disabled={checking || saving}
            onclick={saveAndReload}
          >
            {saving ? "Saving…" : "Save"}
          </Button>
        </div>
      </Card.Content>
    </Card.Root>
  {/if}
</div>
