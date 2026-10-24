import { currentUser } from "#lib/configuration.svelte";
import {
  beginSyncStep,
  globalSyncState,
  resetSyncState,
  SyncStage,
  SyncStep,
} from "#lib/sync.svelte";
import { runWithLimit } from "#lib/utils";
import { getContainer } from "../dependency_injection";
import { getAllGroups, IGroupApi, type Group } from "../shared/group_api";
import type { Model } from "../shared/model_api";
import {
  applySyncResult,
  computeDifferences,
  remoteCounterparts,
  resolveDirection,
  stepDelete,
  withMetaSyncFields,
  type ResourceSet,
} from "./algorithm";

async function finalizeSingleGroupUpload(
  group: Group,
  remoteApi: IGroupApi,
  remoteModelsById: Map<string, Model>,
): Promise<void> {
  const relatedModels = remoteCounterparts(group.models, remoteModelsById);
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
  beginSyncStep(
    isDownload ? SyncStep.Download : SyncStep.Upload,
    toUpload.length,
  );

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

  const relatedModels = remoteCounterparts(localGroup.models, remoteModelsById);

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
  beginSyncStep(SyncStep.UpdateMetadata, toSync.length);

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
  serverModelsById: Map<string, Model>,
  localModelsById: Map<string, Model>,
): Promise<void> {
  const lastSynced = currentUser.lastSync ?? new Date("2000");
  resetSyncState(SyncStage.Groups);
  const localGroupApi = getContainer().require<IGroupApi>(IGroupApi);

  // The two full-list fetches are independent; run them concurrently.
  const [serverGroups, localGroups] = await Promise.all([
    getAllGroups(serverGroupApi),
    getAllGroups(localGroupApi),
  ]);

  const syncState = computeDifferences(
    withMetaSyncFields(localGroups),
    withMetaSyncFields(serverGroups),
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
