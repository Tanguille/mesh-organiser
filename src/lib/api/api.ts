import { isTauri } from "@tauri-apps/api/core";
import { initDemoApis } from "./demo/init";
import { initTauriLocalApis } from "./tauri/init";
import { initWebApi } from "./web/init";
import { initWebShareApi } from "./web_share/init";

export async function initApi(): Promise<void> {
  if (import.meta.env.VITE_API_PLATFORM === "demo") {
    await initDemoApis();
  } else if (import.meta.env.VITE_API_PLATFORM === "web") {
    if (
      document.location.pathname.startsWith("/share/") &&
      !document.location.pathname.endsWith("/share/")
    ) {
      if (!(await initWebShareApi())) {
        throw new Error("Failed to initialize share API");
      }
    } else {
      await initWebApi();
    }
  } else {
    // `tauri dev` serves the same URL as a plain browser; only the webview has IPC.
    if (!isTauri()) {
      await initDemoApis();
      return;
    }
    await initTauriLocalApis();
  }
}
