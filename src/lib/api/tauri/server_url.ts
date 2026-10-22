import { invoke } from "@tauri-apps/api/core";

export async function getServerUrl(): Promise<string | null> {
  return await invoke<string | null>("get_server_url");
}

export async function setServerUrl(url: string): Promise<void> {
  await invoke("set_server_url", { url });
}

export async function clearServerUrl(): Promise<void> {
  await invoke("clear_server_url");
}
