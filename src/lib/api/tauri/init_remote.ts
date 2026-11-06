import { fetch } from "@tauri-apps/plugin-http";
import { getContainer, resetContainer } from "../dependency_injection";
import { IBlobApi } from "../shared/blob_api";
import { IServerRequestApi } from "../shared/server_request_api";
import { ISlicerApi } from "../shared/slicer_api";
import { initWebApi } from "../web/init";
import { MobileRemoteSlicerApi } from "../web/remote_slicer";
import { panicState } from "#lib/configuration.svelte";
import { getServerUrl } from "./server_url";

export async function initTauriRemoteApis(): Promise<void> {
  resetContainer();

  const persisted = await getServerUrl();
  const envUrl =
    (import.meta.env.VITE_MOBILE_SERVER_URL as string | undefined) ?? "";
  const raw = (persisted?.trim() || envUrl.trim() || "").trim();
  const baseUrl = raw.replace(/\/+$/, "");

  if (baseUrl.length === 0) {
    panicState.inPanic = true;
    panicState.message =
      "No remote server URL is configured.\nSet the server URL in the app settings (or VITE_MOBILE_SERVER_URL for development) and restart.";

    return;
  }

  panicState.inPanic = false;

  // The webview origin is not the server, so requests go through plugin-http.
  await initWebApi(baseUrl, fetch);

  const container = getContainer();
  // initWebApi only registers the blob API once the user is authenticated.
  const blob = container.optional<IBlobApi>(IBlobApi);

  if (blob) {
    const request = container.require<IServerRequestApi>(IServerRequestApi);
    container.addSingleton(
      ISlicerApi,
      new MobileRemoteSlicerApi(blob, request),
    );
  }
}
