<script lang="ts">
  import * as Card from "#lib/components/ui/card/index.js";
  import { Input } from "#lib/components/ui/input/index.js";
  import { Label } from "#lib/components/ui/label/index.js";
  import { Button } from "#lib/components/ui/button/index.js";
  import { fetch as tauriFetch } from "@tauri-apps/plugin-http";
  import { HttpMethod } from "#lib/api/shared/server_request_api";
  import { ServerRequestApi } from "#lib/api/web/request";
  import {
    assertMobileReachable,
    isMobileApp,
    normalizeServerBaseUrl,
  } from "#lib/api/mobile/server_url";
  import { getServerUrl, setServerUrl } from "#lib/api/tauri/server_url";
  import { resolve } from "$app/paths";
  import { onMount } from "svelte";
  import { toast } from "svelte-sonner";

  let serverUrl = $state("");
  let checking = $state(false);
  let saving = $state(false);
  let testHint = $state<string | null>(null);

  // Test and Save accept exactly the same URLs.
  function validatedUrl(): string {
    const normalized = normalizeServerBaseUrl(serverUrl);
    assertMobileReachable(normalized);

    return normalized;
  }

  onMount(async () => {
    if (isMobileApp()) {
      serverUrl = (await getServerUrl()) ?? "";
    }
  });

  async function testConnection() {
    testHint = null;
    let base: string;
    try {
      base = validatedUrl();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      toast.error("Invalid server URL.", { description: msg });
      return;
    }

    checking = true;
    try {
      const api = new ServerRequestApi(base, tauriFetch);
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
      await setServerUrl(validatedUrl());
      // Full load at the root so the layout re-runs API init with the new URL.
      location.href = resolve("/");
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      toast.error("Could not save server URL.", { description: msg });
    } finally {
      saving = false;
    }
  }
</script>

<div class="flex h-full items-center justify-center p-4">
  {#if !isMobileApp()}
    <p class="text-center text-sm text-muted-foreground">
      Remote server setup is only available in the Mesh Organiser mobile app.
    </p>
  {:else}
    <Card.Root class="w-full max-w-md">
      <Card.Header>
        <Card.Title>Remote server</Card.Title>
        <Card.Description>
          Enter the HTTPS base URL of your Mesh Organiser server, e.g.
          https://nas.example.com:3000
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
          <p class="text-sm text-muted-foreground">{testHint}</p>
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
