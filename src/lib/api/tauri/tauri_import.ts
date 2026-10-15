import {
  defaultImportState,
  getFileFromUrl,
  globalImportSettings,
  importState,
  navigateToImportPage,
  resetImportState,
} from "#lib/import.svelte";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  ImportStatus,
  type ImportModelSettings,
  type ImportState,
  type ITauriImportApi,
} from "../shared/tauri_import_api";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { toast } from "svelte-sonner";
import { invoke } from "@tauri-apps/api/core";
import { updateSidebarState } from "#lib/sidebar_data.svelte";
import { configuration } from "#lib/configuration.svelte";
import { open } from "@tauri-apps/plugin-dialog";
import { accountLinkData } from "#lib/account_link_data.svelte";
import { BaseDirectory, watch, type WatchEvent } from "@tauri-apps/plugin-fs";
import { MODEL_EXTENSIONS } from "../shared/blob_api";

interface DeepLinkEmit {
  download_url: string;
  source_url: string | null;
}

interface DownloadFinishedEvent {
  path: string;
  url: string;
}

interface DragDropPayload {
  paths?: string[];
}

export interface DownloadResult {
  path: string;
  source_uri: string | null;
}

export interface AccountLinkEmit {
  base_url: string;
  user_name: string;
  link_token: string;
}

export async function downloadFile(url: string): Promise<DownloadResult> {
  return await invoke("download_file", { url: url });
}

async function importModel(
  path: string,
  recursive: boolean,
  delete_imported: boolean,
  import_as_path: boolean,
  origin_url: string | null,
  open_in_slicer: boolean,
): Promise<ImportState> {
  return await invoke("add_model", {
    path: path,
    recursive: recursive,
    deleteImported: delete_imported,
    originUrl: origin_url,
    openInSlicer: open_in_slicer,
    importAsPath: import_as_path,
  });
}

// Applies only the result fields of a finished import. The counter/status
// fields of the global importState are maintained concurrently by the Tauri
// event listeners and must not be clobbered here.
function applyImportResult(target: ImportState, result: ImportState): void {
  target.imported_models.push(...result.imported_models);
  target.origin_url = result.origin_url;
  target.failure_reason = result.failure_reason;
  target.recursive = result.recursive;
  target.delete_after_import = result.delete_after_import;
}

export class TauriImportApi implements ITauriImportApi {
  eventListeners: UnlistenFn[] = [];
  complete: ((value: unknown) => void)[] = [];

  public async startImportProcess(
    paths: string[],
    settings: ImportModelSettings,
  ): Promise<ImportState> {
    let delete_after_import =
      settings.delete_after_import ?? globalImportSettings.delete_after_import;
    const recursive = settings.recursive ?? globalImportSettings.recursive;
    const direct_open_in_slicer = settings.direct_open_in_slicer ?? false;
    const import_as_path =
      settings.import_as_path ?? globalImportSettings.import_as_path;
    const source_url = settings.source_url;

    if (import_as_path) {
      delete_after_import = false;
    }

    if (!paths || paths.length === 0) {
      return defaultImportState();
    }

    resetImportState();
    const localImportState = defaultImportState();

    for (let i = 0; i < paths.length; i++) {
      try {
        console.log("Importing model at path:", paths[i]);
        const importResult = await importModel(
          paths[i],
          recursive,
          delete_after_import,
          import_as_path,
          source_url ?? null,
          direct_open_in_slicer,
        );

        applyImportResult(importState, importResult);
        applyImportResult(localImportState, importResult);

        if (importResult.status === ImportStatus.Failure) {
          importState.status = localImportState.status = ImportStatus.Failure;
          return localImportState;
        }
      } catch (reason: unknown) {
        importState.status = localImportState.status = ImportStatus.Failure;
        const r = reason as {
          error_message?: string;
          error_inner_message?: string;
        };
        importState.failure_reason =
          localImportState.failure_reason = `${r.error_message ?? "unknown"} - ${r.error_inner_message ?? "unknown"}`;
        console.error("Failed to import model:", reason);
        return localImportState;
      }
    }

    await updateSidebarState();
    console.log("Finished importing models:", importState);
    importState.status = localImportState.status = ImportStatus.Finished;
    return localImportState;
  }

  public async setAccountLink(accountLink: AccountLinkEmit): Promise<void> {
    await this.focusWindow();
    accountLinkData.baseUrl = accountLink.base_url;
    accountLinkData.userName = accountLink.user_name;
    accountLinkData.linkToken = accountLink.link_token;
    accountLinkData.showLinkUi = true;
  }

  public async initImportListeners(): Promise<void> {
    resetImportState();
    this.eventListeners.forEach((unlisten) => unlisten());
    this.eventListeners = [];

    // Each handler takes its event's payload; `never` lets the table hold
    // handlers with different payload types.
    const handlers: [string, (payload: never) => unknown][] = [
      [
        "import-status",
        (status: ImportStatus) => (importState.status = status),
      ],
      [
        "import-model-count",
        (count: number) => (importState.imported_models_count = count),
      ],
      [
        "import-thumbnail-count",
        (count: number) => (importState.finished_thumbnails_count = count),
      ],
      [
        "import-model-group",
        (group: string) => (importState.current_importing_group = group),
      ],
      [
        "import-failure",
        (reason: string) => (importState.failure_reason = reason),
      ],
      [
        "import-model-total",
        (total: number) => (importState.model_count = total),
      ],
      [
        "import-all-data",
        // Copies field by field so missing keys reset and unknown keys are ignored.
        (state: ImportState) => {
          importState.imported_models = state.imported_models;
          importState.imported_models_count = state.imported_models_count;
          importState.finished_thumbnails_count =
            state.finished_thumbnails_count;
          importState.model_count = state.model_count;
          importState.status = state.status;
          importState.origin_url = state.origin_url;
          importState.failure_reason = state.failure_reason;
          importState.recursive = state.recursive;
          importState.delete_after_import = state.delete_after_import;
          importState.current_importing_group = undefined;
          navigateToImportPage();
        },
      ],
      ["deep-link", (link: DeepLinkEmit) => this.handleDeepLink(link)],
      [
        "download-started",
        (url: string) => this.handleBuiltInBrowserDownloadStarted(url),
      ],
      [
        "download-finished",
        (download: DownloadFinishedEvent) =>
          this.handleBuiltInBrowserDownloadFinished(download),
      ],
      [
        "tauri://drag-drop",
        (drop?: DragDropPayload) => this.handleDragDropEvent(drop),
      ],
      [
        "account-link",
        (accountLink: AccountLinkEmit) => this.setAccountLink(accountLink),
      ],
    ];

    for (const [eventName, handler] of handlers) {
      this.eventListeners.push(
        await listen<never>(eventName, (event) => handler(event.payload)),
      );
    }

    if (configuration.watch_downloads_folder) {
      this.eventListeners.push(await this.folderWatch());
    }

    globalImportSettings.delete_after_import =
      configuration.default_enabled_delete_after_import;
    globalImportSettings.recursive =
      configuration.default_enabled_recursive_import;
    globalImportSettings.import_as_path =
      configuration.default_enabled_import_as_path;
  }

  async handleFolderWatchEvent(event: WatchEvent): Promise<void> {
    console.log("Folder watch event:", event);
    const paths = event.paths.filter((path) =>
      MODEL_EXTENSIONS.some((extension) =>
        path.toLowerCase().endsWith("." + extension),
      ),
    );

    if (paths.length <= 0) {
      return;
    }

    const evType = event.type as Record<string, { kind?: string } | undefined>;
    if (
      !(
        (Object.hasOwn(evType, "create") && evType.create?.kind === "any") ||
        (Object.hasOwn(evType, "modify") && evType.modify?.kind === "any")
      )
    ) {
      return;
    }

    console.log("Detected new files in Downloads folder to import:", paths);
    // Only files the app downloaded itself are deleted after import.
    const importPromise = this.importRemoteFiles(paths, false);

    toast.promise(importPromise, {
      loading: `Importing ${paths.length} new file(s) from Downloads folder...`,
      success: `Imported ${paths.length} new file(s) from Downloads folder.`,
      error: `Failed to import new file(s) from Downloads folder.`,
    });

    await importPromise;
  }

  // Imports files that arrived from outside the app (deep link, built-in
  // browser, Downloads folder) and opens the import page without waiting.
  importRemoteFiles(
    paths: string[],
    deleteAfterImport: boolean,
    sourceUrl?: string,
  ): Promise<ImportState> {
    const importPromise = this.startImportProcess(paths, {
      source_url: sourceUrl,
      direct_open_in_slicer: configuration.open_slicer_on_remote_model_import,
      delete_after_import: deleteAfterImport,
      recursive: false,
      import_as_path: false,
    });
    navigateToImportPage();

    return importPromise;
  }

  async folderWatch(): Promise<UnlistenFn> {
    return await watch("", (event) => this.handleFolderWatchEvent(event), {
      baseDir: BaseDirectory.Download,
    });
  }

  async focusWindow(): Promise<void> {
    await getCurrentWindow().unminimize();
    await getCurrentWindow().setFocus();
  }

  async handleDeepLinkDownload(event: DeepLinkEmit): Promise<void> {
    const download = await downloadFile(event.download_url);
    // No await, don't want to block on this
    this.importRemoteFiles(
      [download.path],
      true,
      event.source_url ?? download.source_uri ?? undefined,
    );
  }

  async handleDeepLink(event: DeepLinkEmit): Promise<void> {
    const display_url = event.download_url;

    if (configuration.focus_after_link_import) {
      await this.focusWindow();
    }

    await toast.promise(this.handleDeepLinkDownload(event), {
      loading: `Downloading model ${getFileFromUrl(display_url)}`,
      success: `Downloaded model ${getFileFromUrl(display_url)}`,
      error: `Failed to download model ${getFileFromUrl(display_url)}`,
    });
  }

  async handleBuiltInBrowserDownloadStarted(url: string): Promise<void> {
    toast.promise(
      new Promise((resolve) => {
        this.complete.push(resolve);
      }),
      {
        loading: `Downloading model ${getFileFromUrl(url)}`,
        success: `Downloaded model ${getFileFromUrl(url)}`,
        error: `Failed to download model ${getFileFromUrl(url)}`,
      },
    );

    await this.focusWindow();
  }

  // TODO: Move this interaction to the backend. Use the import-data event to trigger the import.
  async handleBuiltInBrowserDownloadFinished(
    event: DownloadFinishedEvent,
  ): Promise<void> {
    if (this.complete.length > 0) {
      this.complete.pop()!(null);
    }

    console.log("download finished (download-finished):", event);
    // No await, don't want to block on this
    this.importRemoteFiles([event.path], true, event.url);
  }

  async handleDragDropEvent(payload?: DragDropPayload): Promise<void> {
    console.log(payload);

    if (!payload?.paths?.length) {
      return;
    }

    navigateToImportPage();
    await this.startImportProcess(payload.paths, {});
  }

  async openFolderForImporting(): Promise<void> {
    await this.openForImporting(true);
  }

  async openFilesForImporting(): Promise<void> {
    await this.openForImporting(false);
  }

  async openForImporting(directory: boolean): Promise<void> {
    let filters = undefined;

    if (!directory) {
      filters = [
        {
          name: "3D Models",
          extensions: [...MODEL_EXTENSIONS, "zip"],
        },
      ];
    }

    const result: string | string[] | null = await open({
      multiple: true,
      directory: directory,
      filters: filters,
    });

    if (!result) {
      return;
    }

    const paths: string[] = typeof result === "string" ? [result] : result;

    await this.startImportProcess(paths, {});
  }
}
