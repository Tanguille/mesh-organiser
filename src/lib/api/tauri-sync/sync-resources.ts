import { currentUser } from "$lib/configuration.svelte";
import {
  globalSyncState,
  resetSyncState,
  SyncStage,
  SyncStep,
} from "$lib/sync.svelte";
import { getContainer } from "../dependency_injection";
import { GroupOrderBy, IGroupApi, type GroupMeta } from "../shared/group_api";
import { IResourceApi, type ResourceMeta } from "../shared/resource_api";
import { computeDifferences, type ResourceSet } from "./algorithm";

async function stepUploadToRemote(
  toUpload: ResourceMeta[],
  localApi: IResourceApi,
  remoteApi: IResourceApi,
  remoteGroups: GroupMeta[],
  isDownload: boolean,
): Promise<void> {
  globalSyncState.step = isDownload ? SyncStep.Download : SyncStep.Upload;
  globalSyncState.processableItems = toUpload.length;
  globalSyncState.processedItems = 0;

  for (const resource of toUpload) {
    const newResource = await remoteApi.addResource(resource.name);
    const localResourceGroups = await localApi.getGroupsForResource(resource);
    const relatedRemoteGroups = remoteGroups.filter((x) =>
      localResourceGroups.some(
        (y) => y.meta.uniqueGlobalId === x.uniqueGlobalId,
      ),
    );

    for (const group of relatedRemoteGroups) {
      await remoteApi.setResourceOnGroup(newResource, group.id);
    }

    resource.id = newResource.id;
    await remoteApi.editResource(resource, true, true);

    globalSyncState.processedItems += 1;
  }
}

async function stepSyncToRemote(
  toSync: ResourceSet<ResourceMeta>[],
  localApi: IResourceApi,
  remoteApi: IResourceApi,
  remoteGroups: GroupMeta[],
  isServerToLocal: boolean,
): Promise<void> {
  globalSyncState.step = SyncStep.UpdateMetadata;
  globalSyncState.processableItems = toSync.length;
  globalSyncState.processedItems = 0;

  for (const resourceSet of toSync) {
    const remoteResource = isServerToLocal
      ? resourceSet.local
      : resourceSet.server;
    const localResource = isServerToLocal
      ? resourceSet.server
      : resourceSet.local;

    const localResourceGroups =
      await localApi.getGroupsForResource(localResource);
    const remoteResourceGroups =
      await remoteApi.getGroupsForResource(remoteResource);
    const relatedRemoteGroups = remoteGroups.filter((x) =>
      localResourceGroups.some(
        (y) => y.meta.uniqueGlobalId === x.uniqueGlobalId,
      ),
    );
    const toRemoveGroups = remoteResourceGroups.filter(
      (x) =>
        !relatedRemoteGroups.some(
          (y) => y.uniqueGlobalId === x.meta.uniqueGlobalId,
        ),
    );

    for (const toRemoveGroup of toRemoveGroups) {
      await remoteApi.setResourceOnGroup(null, toRemoveGroup.meta.id);
    }

    for (const group of relatedRemoteGroups) {
      await remoteApi.setResourceOnGroup(remoteResource, group.id);
    }

    localResource.id = remoteResource.id;
    await remoteApi.editResource(
      localResource,
      true,
      remoteResource.uniqueGlobalId !== localResource.uniqueGlobalId,
    );
    globalSyncState.processedItems += 1;
  }
}

async function deleteFromRemote(
  toDelete: ResourceMeta[],
  remoteApi: IResourceApi,
): Promise<void> {
  globalSyncState.step = SyncStep.Delete;
  globalSyncState.processableItems = toDelete.length;
  globalSyncState.processedItems = 0;

  for (const resource of toDelete) {
    await remoteApi.deleteResource(resource);
    globalSyncState.processedItems += 1;
  }
}

export async function syncResources(
  serverGroupApi: IGroupApi,
  serverResourceApi: IResourceApi,
): Promise<void> {
  const lastSynced = currentUser.lastSync ?? new Date("2000");
  resetSyncState();
  globalSyncState.stage = SyncStage.Resources;
  const localGroupApi = getContainer().require<IGroupApi>(IGroupApi);
  const localResourceApi = getContainer().require<IResourceApi>(IResourceApi);

  const serverGroups = (
    await serverGroupApi.getGroups(
      null,
      null,
      null,
      GroupOrderBy.ModifiedDesc,
      null,
      1,
      9999999,
      false,
    )
  ).map((x) => x.meta);
  const localGroups = (
    await localGroupApi.getGroups(
      null,
      null,
      null,
      GroupOrderBy.ModifiedDesc,
      null,
      1,
      9999999,
      false,
    )
  ).map((x) => x.meta);

  const serverResources = await serverResourceApi.getResources();
  const localResources = await localResourceApi.getResources();

  const syncState = computeDifferences(
    localResources,
    serverResources,
    lastSynced,
  );

  if (syncState.toUpload.length > 0) {
    await stepUploadToRemote(
      syncState.toUpload,
      localResourceApi,
      serverResourceApi,
      serverGroups,
      false,
    );
  }

  if (syncState.toDownload.length > 0) {
    await stepUploadToRemote(
      syncState.toDownload,
      serverResourceApi,
      localResourceApi,
      localGroups,
      true,
    );
  }

  if (syncState.syncToServer.length > 0) {
    await stepSyncToRemote(
      syncState.syncToServer,
      localResourceApi,
      serverResourceApi,
      serverGroups,
      false,
    );
  }

  if (syncState.syncToLocal.length > 0) {
    await stepSyncToRemote(
      syncState.syncToLocal,
      serverResourceApi,
      localResourceApi,
      localGroups,
      true,
    );
  }

  if (syncState.toDeleteServer.length > 0) {
    await deleteFromRemote(syncState.toDeleteServer, serverResourceApi);
  }

  if (syncState.toDeleteLocal.length > 0) {
    await deleteFromRemote(syncState.toDeleteLocal, localResourceApi);
  }
}
