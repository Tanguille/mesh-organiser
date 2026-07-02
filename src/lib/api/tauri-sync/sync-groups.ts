import { currentUser } from "$lib/configuration.svelte";
import {
  globalSyncState,
  resetSyncState,
  SyncStage,
  SyncStep,
} from "$lib/sync.svelte";
import { runWithLimit } from "$lib/utils";
import { getContainer } from "../dependency_injection";
import { getAllGroups, IGroupApi, type Group } from "../shared/group_api";
import { getAllModels, IModelApi, type Model } from "../shared/model_api";
import {
  applySyncResult,
  computeDifferences,
  forceApplyFieldToObject,
  metaFieldExtractor,
  resolveDirection,
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

  await runWithLimit(toUpload, (group) =>
    finalizeSingleGroupUpload(group, remoteApi, remoteModels),
  );
}

async function finalizeSyncToRemote(
  groupSet: ResourceSet<Group>,
  remoteApi: IGroupApi,
  remoteModels: Model[],
  isServerToLocal: boolean,
): Promise<void> {
  const { remote: remoteGroup, local: localGroup } = resolveDirection(
    groupSet,
    isServerToLocal,
  );

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

  await runWithLimit(toSync, (groupSet) =>
    finalizeSyncToRemote(groupSet, remoteApi, remoteModels, isServerToLocal),
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

export async function syncGroups(
  serverModelApi: IModelApi,
  serverGroupApi: IGroupApi,
): Promise<void> {
  const lastSynced = currentUser.lastSync ?? new Date("2000");
  resetSyncState();
  globalSyncState.stage = SyncStage.Groups;
  const localModelApi = getContainer().require<IModelApi>(IModelApi);
  const localGroupApi = getContainer().require<IGroupApi>(IGroupApi);

  // The four full-list fetches are independent; run them concurrently.
  const [serverModels, localModels, serverGroups, localGroups] =
    await Promise.all([
      getAllModels(serverModelApi),
      getAllModels(localModelApi),
      getAllGroups(serverGroupApi),
      getAllGroups(localGroupApi),
    ]);

  const modifiedServerGroups = forceApplyFieldToObject(
    serverGroups,
    metaFieldExtractor,
  );
  const modifiedLocalGroups = forceApplyFieldToObject(
    localGroups,
    metaFieldExtractor,
  );

  const syncState = computeDifferences(
    modifiedLocalGroups,
    modifiedServerGroups,
    lastSynced,
  );

  await applySyncResult(syncState, {
    upload: (toUpload) =>
      stepUploadToRemote(toUpload, serverGroupApi, serverModels, false),
    download: (toDownload) =>
      stepUploadToRemote(toDownload, localGroupApi, localModels, true),
    syncToServer: (toSync) =>
      stepSyncToRemote(toSync, serverGroupApi, serverModels, false),
    syncToLocal: (toSync) =>
      stepSyncToRemote(toSync, localGroupApi, localModels, true),
    deleteServer: (toDelete) => deleteFromRemote(toDelete, serverGroupApi),
    deleteLocal: (toDelete) => deleteFromRemote(toDelete, localGroupApi),
  });
}
