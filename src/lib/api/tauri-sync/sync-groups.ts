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
import type { Model } from "../shared/model_api";
import {
  applySyncResult,
  computeDifferences,
  forceApplyFieldToObject,
  metaFieldExtractor,
  resolveDirection,
  stepDelete,
  type ResourceSet,
} from "./algorithm";

async function finalizeSingleGroupUpload(
  group: Group,
  remoteApi: IGroupApi,
  remoteModelsById: Map<string, Model>,
): Promise<void> {
  const relatedModels = group.models.flatMap(
    (model) => remoteModelsById.get(model.uniqueGlobalId) ?? [],
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
  remoteModelsById: Map<string, Model>,
  isDownload: boolean,
): Promise<void> {
  globalSyncState.step = isDownload ? SyncStep.Download : SyncStep.Upload;
  globalSyncState.processableItems = toUpload.length;
  globalSyncState.processedItems = 0;

  await runWithLimit(toUpload, (group) =>
    finalizeSingleGroupUpload(group, remoteApi, remoteModelsById),
  );
}

async function finalizeSyncToRemote(
  groupSet: ResourceSet<Group>,
  remoteApi: IGroupApi,
  remoteModelsById: Map<string, Model>,
  isServerToLocal: boolean,
): Promise<void> {
  const { remote: remoteGroup, local: localGroup } = resolveDirection(
    groupSet,
    isServerToLocal,
  );

  const relatedModels = localGroup.models.flatMap(
    (model) => remoteModelsById.get(model.uniqueGlobalId) ?? [],
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
  remoteModelsById: Map<string, Model>,
  isServerToLocal: boolean,
): Promise<void> {
  globalSyncState.step = SyncStep.UpdateMetadata;
  globalSyncState.processableItems = toSync.length;
  globalSyncState.processedItems = 0;

  await runWithLimit(toSync, (groupSet) =>
    finalizeSyncToRemote(
      groupSet,
      remoteApi,
      remoteModelsById,
      isServerToLocal,
    ),
  );
}

export async function syncGroups(
  serverGroupApi: IGroupApi,
  serverModels: Model[],
  localModels: Model[],
): Promise<void> {
  const lastSynced = currentUser.lastSync ?? new Date("2000");
  resetSyncState();
  globalSyncState.stage = SyncStage.Groups;
  const localGroupApi = getContainer().require<IGroupApi>(IGroupApi);

  // The two full-list fetches are independent; run them concurrently.
  const [serverGroups, localGroups] = await Promise.all([
    getAllGroups(serverGroupApi),
    getAllGroups(localGroupApi),
  ]);

  // Index the shared model lists once so per-group member resolution is
  // O(group size) instead of a full model-library scan per group.
  const serverModelsById = new Map(
    serverModels.map((x) => [x.uniqueGlobalId, x]),
  );
  const localModelsById = new Map(
    localModels.map((x) => [x.uniqueGlobalId, x]),
  );

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
      stepUploadToRemote(toUpload, serverGroupApi, serverModelsById, false),
    download: (toDownload) =>
      stepUploadToRemote(toDownload, localGroupApi, localModelsById, true),
    syncToServer: (toSync) =>
      stepSyncToRemote(toSync, serverGroupApi, serverModelsById, false),
    syncToLocal: (toSync) =>
      stepSyncToRemote(toSync, localGroupApi, localModelsById, true),
    deleteServer: (toDelete) =>
      stepDelete(toDelete, (group) => serverGroupApi.deleteGroup(group.meta)),
    deleteLocal: (toDelete) =>
      stepDelete(toDelete, (group) => localGroupApi.deleteGroup(group.meta)),
  });
}
