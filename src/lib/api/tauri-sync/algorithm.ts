import { globalSyncState, SyncStep } from "$lib/sync.svelte";

export interface DiffableItem {
  uniqueGlobalId: string;
  lastModified: Date;
}

export interface ResourceSet<T> {
  local: T;
  server: T;
}

// Resolves which side of a ResourceSet is the remote vs local target for the
// current sync direction, so each step doesn't re-derive the swap by hand.
export function resolveDirection<T>(
  set: ResourceSet<T>,
  isServerToLocal: boolean,
): { remote: T; local: T } {
  return {
    remote: isServerToLocal ? set.local : set.server,
    local: isServerToLocal ? set.server : set.local,
  };
}

// Shared DiffableExtractor for items that carry their sync fields on `.meta`
// (groups and labels), replacing the per-file fieldExtractor copies.
export function metaFieldExtractor<T extends { meta: DiffableItem }>(
  item: T,
): DiffableItem {
  return {
    uniqueGlobalId: item.meta.uniqueGlobalId,
    lastModified: item.meta.lastModified,
  };
}

export interface SyncResult<T> {
  toDeleteLocal: T[];
  toDeleteServer: T[];
  toUpload: T[];
  toDownload: T[];
  syncToServer: ResourceSet<T>[];
  syncToLocal: ResourceSet<T>[];
}

export function defaultSyncResult<T>(): SyncResult<T> {
  return {
    toDeleteLocal: [],
    toDeleteServer: [],
    toUpload: [],
    toDownload: [],
    syncToServer: [],
    syncToLocal: [],
  };
}

// Per-branch handlers for the six sync steps, each receiving the matching
// SyncResult bucket. Files bind their own local/remote api arg-swapping inside
// these closures so the runner stays direction-agnostic.
export interface SyncResultHandlers<T> {
  upload: (toUpload: T[]) => Promise<void>;
  download: (toDownload: T[]) => Promise<void>;
  syncToServer: (toSync: ResourceSet<T>[]) => Promise<void>;
  syncToLocal: (toSync: ResourceSet<T>[]) => Promise<void>;
  deleteServer: (toDelete: T[]) => Promise<void>;
  deleteLocal: (toDelete: T[]) => Promise<void>;
}

// Runs the six conditional sync steps in the exact order the three identical
// sync files used inline (groups, labels, resources), each gated by its
// `.length > 0` guard. sync-models stays inline because it interleaves extra
// steps between toDownload and syncToServer.
export async function applySyncResult<T>(
  syncState: SyncResult<T>,
  handlers: SyncResultHandlers<T>,
): Promise<void> {
  if (syncState.toUpload.length > 0) {
    await handlers.upload(syncState.toUpload);
  }

  if (syncState.toDownload.length > 0) {
    await handlers.download(syncState.toDownload);
  }

  if (syncState.syncToServer.length > 0) {
    await handlers.syncToServer(syncState.syncToServer);
  }

  if (syncState.syncToLocal.length > 0) {
    await handlers.syncToLocal(syncState.syncToLocal);
  }

  if (syncState.toDeleteServer.length > 0) {
    await handlers.deleteServer(syncState.toDeleteServer);
  }

  if (syncState.toDeleteLocal.length > 0) {
    await handlers.deleteLocal(syncState.toDeleteLocal);
  }
}

// Shared SyncStep.Delete progress loop; the per-entity delete call is the only
// part that differed between the three sync files' deleteFromRemote copies.
export async function stepDelete<T>(
  toDelete: T[],
  deleteItem: (item: T) => Promise<void>,
): Promise<void> {
  globalSyncState.step = SyncStep.Delete;
  globalSyncState.processableItems = toDelete.length;
  globalSyncState.processedItems = 0;

  for (const item of toDelete) {
    await deleteItem(item);
    globalSyncState.processedItems += 1;
  }
}

interface DiffableExtractor<T> {
  (item: T): DiffableItem;
}

export function forceApplyFieldToObject<T>(
  objects: T[],
  fieldExtractor: DiffableExtractor<T>,
): (T & DiffableItem)[] {
  return objects.map((obj) => {
    return {
      ...obj,
      ...fieldExtractor(obj),
    };
  });
}

export function computeDifferences<T extends DiffableItem>(
  localItems: T[],
  serverItems: T[],
  lastSynced: Date,
): SyncResult<T> {
  const result = defaultSyncResult<T>();

  // Index both sides once so the diff is O(n + m) instead of a linear scan of
  // the opposite list per item.
  const serverById = new Map(serverItems.map((x) => [x.uniqueGlobalId, x]));
  const localIds = new Set(localItems.map((x) => x.uniqueGlobalId));

  for (const localItem of localItems) {
    const equivalentServerModel = serverById.get(localItem.uniqueGlobalId);

    if (!equivalentServerModel) {
      if (localItem.lastModified.getTime() < lastSynced.getTime()) {
        result.toDeleteLocal.push(localItem);
      } else {
        result.toUpload.push(localItem);
      }
    } else if (
      equivalentServerModel.lastModified.getTime() ===
      localItem.lastModified.getTime()
    ) {
      // In sync
    } else if (
      equivalentServerModel.lastModified.getTime() <
      localItem.lastModified.getTime()
    ) {
      result.syncToServer.push({
        local: localItem,
        server: equivalentServerModel,
      });
    } else {
      result.syncToLocal.push({
        local: localItem,
        server: equivalentServerModel,
      });
    }
  }

  for (const serverItem of serverItems) {
    if (!localIds.has(serverItem.uniqueGlobalId)) {
      if (serverItem.lastModified.getTime() < lastSynced.getTime()) {
        result.toDeleteServer.push(serverItem);
      } else {
        result.toDownload.push(serverItem);
      }
    }
  }

  return result;
}
