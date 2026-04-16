import { currentUser } from "$lib/configuration.svelte";
import {
  globalSyncState,
  resetSyncState,
  SyncStage,
  SyncStep,
} from "$lib/sync.svelte";
import { invoke } from "@tauri-apps/api/core";
import { getContainer } from "../dependency_injection";
import { IModelApi, ModelOrderBy, type Model } from "../shared/model_api";
import type {
  UploadResult,
  DirectoryScanModel,
} from "../tauri-online/tauri_import";
import { importState } from "$lib/import.svelte";
import {
  ImportStatus,
  ITauriImportApi,
  type ImportState,
} from "../shared/tauri_import_api";
import { IGroupApi } from "../shared/group_api";
import type { IBlobApi } from "../shared/blob_api";
import { downloadFile } from "../tauri/tauri_import";
import { computeDifferences, type ResourceSet } from "./algorithm";
import { runGeneratorWithLimit } from "../web/web_import";

interface BlobPath {
  blob_id: number;
  blob_path: string;
}

/** Resolves the new local model id after a single-file import, or null if import did not yield one. */
export function localModelIdFromSingleFileImport(
  state: ImportState,
): number | null {
  if (state.status === ImportStatus.Failure) {
    return null;
  }

  const firstSet = state.imported_models[0];
  const id = firstSet?.model_ids?.[0];

  return id ?? null;
}

async function finalizeSingleModelUpload(
  paths: BlobPath[],
  upload: DirectoryScanModel,
  serverModelApi: IModelApi,
  serverGroupApi: IGroupApi,
  toUpload: Model[],
): Promise<void> {
  const blobId = paths.find((x) => x.blob_path === upload.path)!.blob_id;
  const model = toUpload.find((x) => x.blob.id === blobId)!;

  model.id = upload.model_ids![0];
  await serverGroupApi.removeModelsFromGroup([model]);
  await serverModelApi.editModel(model, true, true);
  globalSyncState.processedItems += 1;
}

async function stepUpload(
  toUpload: Model[],
  serverModelApi: IModelApi,
  serverGroupApi: IGroupApi,
): Promise<void> {
  globalSyncState.step = SyncStep.Upload;
  globalSyncState.processableItems = toUpload.length;
  globalSyncState.processedItems = 0;

  const paths = await invoke<BlobPath[]>("blobs_to_path", {
    blobIds: toUpload.map((x) => x.blob.id),
  });
  const uploads = await invoke<UploadResult>("upload_models_to_remote_server", {
    paths: paths.map((x) => x.blob_path),
    recursive: false,
    openInSlicer: false,
    sourceUrl: null,
  });
  importState.status = ImportStatus.Idle;

  function* finalizeUploadPromises(
    paths: BlobPath[],
    uploads: UploadResult,
    serverModelApi: IModelApi,
    serverGroupApi: IGroupApi,
    toUpload: Model[],
  ) {
    for (const upload of uploads.uploaded_models) {
      yield finalizeSingleModelUpload(
        paths,
        upload,
        serverModelApi,
        serverGroupApi,
        toUpload,
      );
    }
  }

  // Serialize finalize (detach + edit per uploaded model); parallel calls can reorder remote
  // updates during bulk sync — same pattern as single-flight download.
  await runGeneratorWithLimit(
    finalizeUploadPromises(
      paths,
      uploads,
      serverModelApi,
      serverGroupApi,
      toUpload,
    ),
    1,
  );
}

async function downloadSingleModel(
  serverModel: Model,
  serverBlobApi: IBlobApi,
  localModelApi: IModelApi,
  localImportApi: ITauriImportApi,
): Promise<void> {
  const downloadUrl = await serverBlobApi.getBlobDownloadUrl(serverModel.blob);
  const download = await downloadFile(downloadUrl);

  // Full local import pipeline for one file (thumbnails, DB, etc.); a slimmer “attach blob” API would avoid duplicate work if we add it later.
  const localImportState = await localImportApi.startImportProcess(
    [download.path],
    {
      delete_after_import: true,
      recursive: false,
      direct_open_in_slicer: false,
      import_as_path: false,
    },
  );

  const id = localModelIdFromSingleFileImport(localImportState);
  if (id === null) {
    throw new Error(
      "Sync download: import did not produce a local model id (empty result or import failure).",
    );
  }

  serverModel.id = id;
  await localModelApi.editModel(serverModel, true, true);
  globalSyncState.processedItems += 1;
  importState.status = ImportStatus.Idle;
}

async function stepDownload(
  toDownload: Model[],
  serverBlobApi: IBlobApi,
  localModelApi: IModelApi,
  localImportApi: ITauriImportApi,
): Promise<void> {
  globalSyncState.step = SyncStep.Download;
  globalSyncState.processableItems = toDownload.length;
  globalSyncState.processedItems = 0;

  function* downloadPromises(
    toDownload: Model[],
    serverBlobApi: IBlobApi,
    localModelApi: IModelApi,
    localImportApi: ITauriImportApi,
  ) {
    for (const serverModel of toDownload) {
      yield downloadSingleModel(
        serverModel,
        serverBlobApi,
        localModelApi,
        localImportApi,
      );
    }
  }

  // Single-flight: each download runs the full Tauri import pipeline, which resets shared
  // `importState`. Parallel downloads would race that global state (see code review).
  await runGeneratorWithLimit(
    downloadPromises(toDownload, serverBlobApi, localModelApi, localImportApi),
    1,
  );
}

async function syncSingleModelToServer(
  syncToServer: ResourceSet<Model>,
  serverModelApi: IModelApi,
  isServerToLocal: boolean,
): Promise<void> {
  const serverModel = isServerToLocal
    ? syncToServer.local
    : syncToServer.server;
  const localModel = isServerToLocal ? syncToServer.server : syncToServer.local;

  localModel.id = serverModel.id;
  await serverModelApi.editModel(
    localModel,
    true,
    serverModel.uniqueGlobalId !== localModel.uniqueGlobalId,
  );
  globalSyncState.processedItems += 1;
}

async function stepSyncToRemote(
  syncToServer: ResourceSet<Model>[],
  serverModelApi: IModelApi,
  isServerToLocal: boolean,
): Promise<void> {
  globalSyncState.step = SyncStep.UpdateMetadata;
  globalSyncState.processableItems = syncToServer.length;
  globalSyncState.processedItems = 0;

  function* syncToServerPromises(
    syncToServer: ResourceSet<Model>[],
    serverModelApi: IModelApi,
    isServerToLocal: boolean,
  ) {
    for (const modelSet of syncToServer) {
      yield syncSingleModelToServer(modelSet, serverModelApi, isServerToLocal);
    }
  }

  await runGeneratorWithLimit(
    syncToServerPromises(syncToServer, serverModelApi, isServerToLocal),
    4,
  );
}

async function stepDeleteFromRemote(
  toDelete: Model[],
  remoteApi: IModelApi,
): Promise<void> {
  globalSyncState.step = SyncStep.Delete;
  globalSyncState.processableItems = toDelete.length;
  globalSyncState.processedItems = 0;

  await remoteApi.deleteModels(toDelete);
  globalSyncState.processedItems = toDelete.length;
}

export async function syncModels(
  serverModelApi: IModelApi,
  serverGroupApi: IGroupApi,
  serverBlobApi: IBlobApi,
): Promise<void> {
  const lastSynced = currentUser.lastSync ?? new Date("2000");
  resetSyncState();
  globalSyncState.stage = SyncStage.Models;
  const localModelApi = getContainer().require<IModelApi>(IModelApi);
  const localGroupApi = getContainer().require<IGroupApi>(IGroupApi);
  const localImportApi =
    getContainer().require<ITauriImportApi>(ITauriImportApi);

  const serverModels = await serverModelApi.getModels(
    null,
    null,
    null,
    ModelOrderBy.ModifiedDesc,
    null,
    1,
    9999999,
    null,
  );
  const localModels = await localModelApi.getModels(
    null,
    null,
    null,
    ModelOrderBy.ModifiedDesc,
    null,
    1,
    9999999,
    null,
  );

  const syncState = computeDifferences(localModels, serverModels, lastSynced);
  const removeGroupFromModelsLocal = [];
  const removeGroupFromModelsServer = [];

  for (const upload of Array.from(syncState.toUpload)) {
    const relatedDownload = syncState.toDownload.find(
      (serverModel) => serverModel.blob.sha256 === upload.blob.sha256,
    );

    if (!relatedDownload) {
      continue;
    }

    // If we get here, we're in some kind of in progress sync state. Now to figure out which!
    syncState.toDownload.splice(
      syncState.toDownload.indexOf(relatedDownload),
      1,
    );
    syncState.toUpload.splice(syncState.toUpload.indexOf(upload), 1);

    if (
      upload.lastModified.getTime() > relatedDownload.lastModified.getTime()
    ) {
      // If the local model is newer, it's likely that the server download got cancelled mid-way through
      syncState.syncToLocal.push({
        local: upload,
        server: relatedDownload,
      });
      removeGroupFromModelsLocal.push(upload);
    } else {
      // If the server model is newer, it's likely that the local upload got cancelled mid-way through
      syncState.syncToServer.push({
        local: upload,
        server: relatedDownload,
      });
      removeGroupFromModelsServer.push(relatedDownload);
    }
  }

  if (syncState.toUpload.length > 0) {
    await stepUpload(syncState.toUpload, serverModelApi, serverGroupApi);
  }

  if (syncState.toDownload.length > 0) {
    await stepDownload(
      syncState.toDownload,
      serverBlobApi,
      localModelApi,
      localImportApi,
    );
  }

  if (removeGroupFromModelsLocal.length > 0) {
    await localGroupApi.removeModelsFromGroup(removeGroupFromModelsLocal);
  }

  if (removeGroupFromModelsServer.length > 0) {
    await serverGroupApi.removeModelsFromGroup(removeGroupFromModelsServer);
  }

  if (syncState.syncToServer.length > 0) {
    await stepSyncToRemote(syncState.syncToServer, serverModelApi, false);
  }

  if (syncState.syncToLocal.length > 0) {
    await stepSyncToRemote(syncState.syncToLocal, localModelApi, true);
  }

  if (syncState.toDeleteServer.length > 0) {
    await stepDeleteFromRemote(syncState.toDeleteServer, serverModelApi);
  }

  if (syncState.toDeleteLocal.length > 0) {
    await stepDeleteFromRemote(syncState.toDeleteLocal, localModelApi);
  }
}
