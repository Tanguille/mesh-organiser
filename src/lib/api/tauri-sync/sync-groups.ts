import { currentUser } from "$lib/configuration.svelte";
import {
  globalSyncState,
  resetSyncState,
  SyncStage,
  SyncStep,
} from "$lib/sync.svelte";
import { getContainer } from "../dependency_injection";
import { GroupOrderBy, IGroupApi, type Group } from "../shared/group_api";
import { IModelApi, ModelOrderBy, type Model } from "../shared/model_api";
import { runGeneratorWithLimit } from "../web/web_import";
import {
  computeDifferences,
  forceApplyFieldToObject,
  type DiffableItem,
  type ResourceSet,
} from "./algorithm";

async function finalizeSingleGroupUpload(
  group: Group,
  remoteApi: IGroupApi,
  remoteModels: Model[],
): Promise<void> {
  const relatedModels = remoteModels.filter((x) =>
    group.models.some((y) => y.uniqueGlobalId === x.uniqueGlobalId),
  );
  const newGroup = await remoteApi.addGroup(group.meta.name);
  group.meta.id = newGroup.id;
  await remoteApi.addModelsToGroup(group.meta, relatedModels);
  await remoteApi.editGroup(group.meta, true, true);
  globalSyncState.processedItems += 1;
}

async function stepUploadToRemote(
  toUpload: Group[],
  remoteApi: IGroupApi,
  remoteModels: Model[],
  isDownload: boolean,
): Promise<void> {
  globalSyncState.step = isDownload ? SyncStep.Download : SyncStep.Upload;
  globalSyncState.processableItems = toUpload.length;
  globalSyncState.processedItems = 0;

  function* finalizeUploadPromises(
    toUpload: Group[],
    remoteApi: IGroupApi,
    remoteModels: Model[],
  ) {
    for (const group of toUpload) {
      yield finalizeSingleGroupUpload(group, remoteApi, remoteModels);
    }
  }

  await runGeneratorWithLimit(
    finalizeUploadPromises(toUpload, remoteApi, remoteModels),
    4,
  );
}

async function finalizeSyncToRemote(
  groupSet: ResourceSet<Group>,
  remoteApi: IGroupApi,
  remoteModels: Model[],
  isServerToLocal: boolean,
): Promise<void> {
  const remoteGroup = isServerToLocal ? groupSet.local : groupSet.server;
  const localGroup = isServerToLocal ? groupSet.server : groupSet.local;

  const relatedModels = remoteModels.filter((x) =>
    localGroup.models.some((y) => y.uniqueGlobalId === x.uniqueGlobalId),
  );

  await remoteApi.removeModelsFromGroup(remoteGroup.models);
  await remoteApi.addModelsToGroup(remoteGroup.meta, relatedModels);
  localGroup.meta.id = remoteGroup.meta.id;
  await remoteApi.editGroup(
    localGroup.meta,
    true,
    remoteGroup.meta.uniqueGlobalId !== localGroup.meta.uniqueGlobalId,
  );
  globalSyncState.processedItems += 1;
}

async function stepSyncToRemote(
  toSync: ResourceSet<Group>[],
  remoteApi: IGroupApi,
  remoteModels: Model[],
  isServerToLocal: boolean,
): Promise<void> {
  globalSyncState.step = SyncStep.UpdateMetadata;
  globalSyncState.processableItems = toSync.length;
  globalSyncState.processedItems = 0;

  function* finalizeSyncPromises(
    toSync: ResourceSet<Group>[],
    remoteApi: IGroupApi,
    remoteModels: Model[],
    isServerToLocal: boolean,
  ) {
    for (const groupSet of toSync) {
      yield finalizeSyncToRemote(
        groupSet,
        remoteApi,
        remoteModels,
        isServerToLocal,
      );
    }
  }

  await runGeneratorWithLimit(
    finalizeSyncPromises(toSync, remoteApi, remoteModels, isServerToLocal),
    4,
  );
}

async function deleteFromRemote(
  toDelete: Group[],
  localApi: IGroupApi,
): Promise<void> {
  globalSyncState.step = SyncStep.Delete;
  globalSyncState.processableItems = toDelete.length;
  globalSyncState.processedItems = 0;

  for (const group of toDelete) {
    await localApi.deleteGroup(group.meta);
    globalSyncState.processedItems += 1;
  }
}

function fieldExtractor(group: Group): DiffableItem {
  return {
    uniqueGlobalId: group.meta.uniqueGlobalId,
    lastModified: group.meta.lastModified,
  };
}

export async function syncGroups(
  serverModelApi: IModelApi,
  serverGroupApi: IGroupApi,
): Promise<void> {
  const lastSynced = currentUser.lastSync ?? new Date("2000");
  resetSyncState();
  globalSyncState.stage = SyncStage.Groups;
  const localModelApi = getContainer().require<IModelApi>(IModelApi);
  const localGroupApi = getContainer().require<IGroupApi>(IGroupApi);

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

  const serverGroups = await serverGroupApi.getGroups(
    null,
    null,
    null,
    GroupOrderBy.ModifiedDesc,
    null,
    1,
    9999999,
    false,
  );
  const localGroups = await localGroupApi.getGroups(
    null,
    null,
    null,
    GroupOrderBy.ModifiedDesc,
    null,
    1,
    9999999,
    false,
  );

  const modifiedServerGroups = forceApplyFieldToObject(
    serverGroups,
    fieldExtractor,
  );
  const modifiedLocalGroups = forceApplyFieldToObject(
    localGroups,
    fieldExtractor,
  );

  const syncState = computeDifferences(
    modifiedLocalGroups,
    modifiedServerGroups,
    lastSynced,
  );

  if (syncState.toUpload.length > 0) {
    await stepUploadToRemote(
      syncState.toUpload,
      serverGroupApi,
      serverModels,
      false,
    );
  }

  if (syncState.toDownload.length > 0) {
    await stepUploadToRemote(
      syncState.toDownload,
      localGroupApi,
      localModels,
      true,
    );
  }

  if (syncState.syncToServer.length > 0) {
    await stepSyncToRemote(
      syncState.syncToServer,
      serverGroupApi,
      serverModels,
      false,
    );
  }

  if (syncState.syncToLocal.length > 0) {
    await stepSyncToRemote(
      syncState.syncToLocal,
      localGroupApi,
      localModels,
      true,
    );
  }

  if (syncState.toDeleteServer.length > 0) {
    await deleteFromRemote(syncState.toDeleteServer, serverGroupApi);
  }

  if (syncState.toDeleteLocal.length > 0) {
    await deleteFromRemote(syncState.toDeleteLocal, localGroupApi);
  }
}
