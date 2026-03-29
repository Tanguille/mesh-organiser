<script lang="ts">
  import "../app.css";
  import * as Sidebar from "$lib/components/ui/sidebar/index.js";
  import AppSidebar from "$lib/components/app-sidebar.svelte";
  import { ModeWatcher } from "mode-watcher";
  import { onMount } from "svelte";
  import { Toaster } from "$lib/components/ui/sonner/index.js";
  import { toast } from "svelte-sonner";
  import { goto } from "$app/navigation";
  import { resolve } from "$app/paths";
  import { IsMobile } from "$lib/hooks/is-mobile.svelte";
  import { setTheme } from "$lib/theme";
  import UpdatePopup from "$lib/components/view/tauri-update-popup.svelte";
  import DragSelectedModelsRoot from "$lib/components/view/drag-selected-models-root.svelte";
  import { initApi } from "$lib/api/api";
  import {
    configuration,
    configurationMeta,
    panicState,
    scheduleConfigurationPersist,
  } from "$lib/configuration.svelte";
  import { updateSidebarState } from "$lib/sidebar_data.svelte";
  import { updateState } from "$lib/update_data.svelte";
  import Spinner from "$lib/components/view/spinner.svelte";
  import { getContainer } from "$lib/api/dependency_injection";
  import { ISidebarStateApi } from "$lib/api/shared/sidebar_state_api";
  import { IUserApi } from "$lib/api/shared/user_api";
  import { accountLinkData } from "$lib/account_link_data.svelte";
  import WebAccountLinkPopup from "$lib/components/view/web-account-link-popup.svelte";
  import BottomNav from "$lib/components/mobile/bottom-nav.svelte";

  let { children } = $props();
  let initializationDone = $state(false);
  let hasSidebar = $state(true);
  const isMobile = new IsMobile();

  /** Vite 8 dev client can race HMR transport `send` before `connect` (see `vite/dist/client/client.mjs`). */
  function isViteDevClientNoise(message: string): boolean {
    return (
      message.includes("send was called before connect") ||
      message.includes("invoke was called before connect")
    );
  }

  function rejectionPlainMessage(reason: unknown): string {
    if (typeof reason === "object" && reason !== null && "message" in reason) {
      return String((reason as { message?: unknown }).message);
    }
    return String(reason);
  }

  function isApiRejectionPayload(
    reason: unknown,
  ): reason is { error_message: string; error_inner_message: string } {
    if (typeof reason !== "object" || reason === null) {
      return false;
    }
    const r = reason as Record<string, unknown>;
    return (
      typeof r.error_message === "string" &&
      typeof r.error_inner_message === "string"
    );
  }

  function handleWindowError(event: Event) {
    const message =
      event instanceof ErrorEvent ? event.message : "Unknown error";
    toast.error(`Error: ${message}`);
  }

  function handleUnhandledRejection(event: PromiseRejectionEvent) {
    const plain = rejectionPlainMessage(event.reason);
    if (import.meta.env.DEV && isViteDevClientNoise(plain)) {
      console.warn("[vite dev]", plain);
      event.preventDefault();
      return;
    }

    const reason = event.reason;
    if (isApiRejectionPayload(reason)) {
      toast.error(reason.error_message, {
        description: reason.error_inner_message,
      });
    } else {
      toast.error("An unknown error occurred.", {
        description: plain,
      });
    }
  }

  onMount(async () => {
    initializationDone = false;

    try {
      await initApi();
      configurationMeta.configurationLoaded = true;
      await setTheme(configuration.theme);

      let userApi = getContainer().optional<IUserApi>(IUserApi);

      if (panicState.inPanic) {
        await goto(resolve("/panic"));
      } else if (userApi && !(await userApi.isAuthenticated())) {
        await goto(resolve("/login"));
      }

      if (panicState.inPanic) {
        await goto(resolve("/panic"));
      }

      if (getContainer().optional<ISidebarStateApi>(ISidebarStateApi) == null) {
        hasSidebar = false;
      } else if (isMobile.current) {
        // Hide sidebar on mobile, use bottom nav instead
        hasSidebar = false;
      } else {
        await updateSidebarState();
      }

      initializationDone = true;
    } catch (e) {
      console.error("Application initialization failed:", e);
      toast.error("Application failed to initialize", {
        description:
          e instanceof Error
            ? e.message
            : typeof e === "string"
              ? e
              : String(e),
      });
    }
  });

  const save_configuration_debounce_ms = 400;

  const configuration_autosave_snapshot = $derived.by(() => {
    if (!initializationDone || !configurationMeta.configurationLoaded) {
      return undefined;
    }
    return $state.snapshot(configuration);
  });

  $effect(() => {
    const snapshot = configuration_autosave_snapshot;
    if (snapshot === undefined) {
      return;
    }

    return scheduleConfigurationPersist(
      snapshot,
      save_configuration_debounce_ms,
    );
  });
</script>

<svelte:window
  onerror={handleWindowError}
  onunhandledrejection={handleUnhandledRejection}
/>
<ModeWatcher />
<Toaster />
{#if initializationDone}
  <DragSelectedModelsRoot class="h-full w-full">
    <Sidebar.Provider class="h-full w-full">
      {#if hasSidebar}
        <AppSidebar />
      {/if}
      <main class="flex h-full flex-1 flex-row" style="min-width: 0;">
        {#if isMobile.current && hasSidebar}
          <Sidebar.Trigger
            class="absolute z-10 aspect-square h-10 w-10 bg-background"
          />
        {/if}
        <div class="flex-1 pb-16 pl-2" style="min-width: 0;">
          {@render children?.()}
        </div>
      </main>
      {#if isMobile.current && !hasSidebar}
        <BottomNav />
      {/if}
      {#if updateState.update}
        <UpdatePopup
          update={updateState.update}
          onDismiss={() => (updateState.update = null)}
        />
      {/if}
      {#if accountLinkData.showLinkUi}
        <WebAccountLinkPopup
          data={accountLinkData}
          onDismiss={() => (accountLinkData.showLinkUi = false)}
        />
      {/if}
    </Sidebar.Provider>
  </DragSelectedModelsRoot>
{:else}
  <div class="flex h-full w-full items-center justify-center">
    <Spinner />
  </div>
{/if}
