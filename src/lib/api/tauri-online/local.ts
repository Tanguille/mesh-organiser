import { invoke } from "@tauri-apps/api/core";
import type { Model } from "../shared/model_api";
import { LocalApi } from "../tauri/local";

// TODO: Split this off into chunks. We don't need the data picker, appdatadir or max parallelism here!
export class OnlineLocalApi extends LocalApi {
  private baseUrl: string;
  private userId: number;
  private userHash: string;

  constructor(
    appDataDir: string,
    maxParallelism: number,
    baseUrl: string,
    userId: number,
    userHash: string,
  ) {
    super(appDataDir, maxParallelism);
    this.baseUrl = baseUrl;
    this.userId = userId;
    this.userHash = userHash;
  }

  async openInFolder(models: Model[], asZip: boolean): Promise<void> {
    await invoke("download_files_and_open_in_folder", {
      sha256s: models.map((m) => m.blob.sha256),
      baseUrl: this.baseUrl,
      userId: this.userId,
      userHash: this.userHash,
      asZip: asZip,
    });
  }
}
