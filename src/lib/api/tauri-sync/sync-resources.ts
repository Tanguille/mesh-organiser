import { currentUser } from "$lib/configuration.svelte";
import {
  globalSyncState,
  resetSyncState,
  SyncStage,
  SyncStep,
} from "$lib/sync.svelte";
import { getContainer } from "../dependency_injection";
import { getAllGroups, IGroupApi, type GroupMeta } from "../shared/group_api";
import { IResourceApi, type ResourceMeta } from "../shared/resource_api";
import {
  applySyncResult,
  computeDifferences,
  resolveDirection,
  stepDelete,
  type ResourceSet,
} from "./algorithm";

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
    const { remote: remoteResource, local: localResource } = resolveDirection(
      resourceSet,
      isServerToLocal,
    );

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

export async function syncResources(
  serverGroupApi: IGroupApi,
  serverResourceApi: IResourceApi,
): Promise<void> {
  const lastSynced = currentUser.lastSync ?? new Date("2000");
  resetSyncState();
  globalSyncState.stage = SyncStage.Resources;
  const localGroupApi = getContainer().require<IGroupApi>(IGroupApi);
  const localResourceApi = getContainer().require<IResourceApi>(IResourceApi);

  // The four full-list fetches are independent; run them concurrently.
  const [serverGroupList, localGroupList, serverResources, localResources] =
    await Promise.all([
      getAllGroups(serverGroupApi),
      getAllGroups(localGroupApi),
      serverResourceApi.getResources(),
      localResourceApi.getResources(),
    ]);
  const serverGroups = serverGroupList.map((x) => x.meta);
  const localGroups = localGroupList.map((x) => x.meta);

  const syncState = computeDifferences(
    localResources,
    serverResources,
    lastSynced,
  );

  await applySyncResult(syncState, {
    upload: (toUpload) =>
      stepUploadToRemote(
        toUpload,
        localResourceApi,
        serverResourceApi,
        serverGroups,
        false,
      ),
    download: (toDownload) =>
      stepUploadToRemote(
        toDownload,
        serverResourceApi,
        localResourceApi,
        localGroups,
        true,
      ),
    syncToServer: (toSync) =>
      stepSyncToRemote(
        toSync,
        localResourceApi,
        serverResourceApi,
        serverGroups,
        false,
      ),
    syncToLocal: (toSync) =>
      stepSyncToRemote(
        toSync,
        serverResourceApi,
        localResourceApi,
        localGroups,
        true,
      ),
    deleteServer: (toDelete) =>
      stepDelete(toDelete, (resource) =>
        serverResourceApi.deleteResource(resource),
      ),
    deleteLocal: (toDelete) =>
      stepDelete(toDelete, (resource) =>
        localResourceApi.deleteResource(resource),
      ),
  });
}
