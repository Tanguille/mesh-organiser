import { currentUser } from "$lib/configuration.svelte";
import {
  globalSyncState,
  resetSyncState,
  SyncStage,
  SyncStep,
} from "$lib/sync.svelte";
import { getContainer } from "../dependency_injection";
import { ILabelApi, type Label } from "../shared/label_api";
import { IModelApi, ModelOrderBy, type Model } from "../shared/model_api";
import {
  applySyncResult,
  computeDifferences,
  forceApplyFieldToObject,
  getAllModels,
  metaFieldExtractor,
  resolveDirection,
  type ResourceSet,
} from "./algorithm";

async function stepUploadToRemote(
  toUpload: Label[],
  localApi: ILabelApi,
  remoteApi: ILabelApi,
  localModelApi: IModelApi,
  remoteModels: Model[],
  isDownload: boolean,
): Promise<void> {
  globalSyncState.step = isDownload ? SyncStep.Download : SyncStep.Upload;
  globalSyncState.processableItems = toUpload.length;
  globalSyncState.processedItems = 0;

  for (const label of toUpload) {
    const newLabel = await remoteApi.addLabel(
      label.meta.name,
      label.meta.color,
    );

    const keywords = await localApi.getKeywordsForLabel(label.meta);
    remoteApi.setKeywordsOnLabel(newLabel, keywords);

    const localModelsForLabel = await localModelApi.getModels(
      null,
      null,
      [label.meta.id],
      ModelOrderBy.ModifiedDesc,
      null,
      1,
      9999999,
      null,
    );
    const relatedRemoteModels = remoteModels.filter((x) =>
      localModelsForLabel.some((y) => y.uniqueGlobalId === x.uniqueGlobalId),
    );
    await remoteApi.addLabelToModels(newLabel, relatedRemoteModels);

    const relatedRemoteChildLabels = (await remoteApi.getLabels(false))
      .map((x) => x.meta)
      .filter((x) =>
        label.children.some((y) => y.uniqueGlobalId === x.uniqueGlobalId),
      );
    await remoteApi.setChildrenOnLabel(newLabel, relatedRemoteChildLabels);

    label.meta.id = newLabel.id;
    await remoteApi.editLabel(label.meta, true, true);
    globalSyncState.processedItems += 1;
  }
}

async function stepSyncToRemote(
  toSync: ResourceSet<Label>[],
  localApi: ILabelApi,
  remoteApi: ILabelApi,
  localModelApi: IModelApi,
  remoteModelApi: IModelApi,
  remoteModels: Model[],
  isServerToLocal: boolean,
): Promise<void> {
  globalSyncState.step = SyncStep.UpdateMetadata;
  globalSyncState.processableItems = toSync.length;
  globalSyncState.processedItems = 0;

  for (const labelSet of toSync) {
    const { remote: remoteLabel, local: localLabel } = resolveDirection(
      labelSet,
      isServerToLocal,
    );

    const keywords = await localApi.getKeywordsForLabel(localLabel.meta);
    await remoteApi.setKeywordsOnLabel(remoteLabel.meta, keywords);

    const localModelsForLabel = await localModelApi.getModels(
      null,
      null,
      [localLabel.meta.id],
      ModelOrderBy.ModifiedDesc,
      null,
      1,
      9999999,
      null,
    );
    const remoteModelsForLabel = await remoteModelApi.getModels(
      null,
      null,
      [remoteLabel.meta.id],
      ModelOrderBy.ModifiedDesc,
      null,
      1,
      9999999,
      null,
    );
    await remoteApi.removeLabelFromModels(
      remoteLabel.meta,
      remoteModelsForLabel,
    );
    const relatedRemoteModels = remoteModels.filter((x) =>
      localModelsForLabel.some((y) => y.uniqueGlobalId === x.uniqueGlobalId),
    );
    await remoteApi.addLabelToModels(remoteLabel.meta, relatedRemoteModels);

    const relatedRemoteChildLabels = (await remoteApi.getLabels(false))
      .map((x) => x.meta)
      .filter((x) =>
        localLabel.children.some((y) => y.uniqueGlobalId === x.uniqueGlobalId),
      );

    await remoteApi.setChildrenOnLabel(
      remoteLabel.meta,
      relatedRemoteChildLabels,
    );

    localLabel.meta.id = remoteLabel.meta.id;
    await remoteApi.editLabel(
      localLabel.meta,
      true,
      remoteLabel.meta.uniqueGlobalId !== localLabel.meta.uniqueGlobalId,
    );
    globalSyncState.processedItems += 1;
  }
}

async function deleteFromRemote(
  toDelete: Label[],
  remoteApi: ILabelApi,
): Promise<void> {
  globalSyncState.step = SyncStep.Delete;
  globalSyncState.processableItems = toDelete.length;
  globalSyncState.processedItems = 0;

  for (const label of toDelete) {
    await remoteApi.deleteLabel(label.meta);
    globalSyncState.processedItems += 1;
  }
}

export async function syncLabels(
  serverModelApi: IModelApi,
  serverLabelApi: ILabelApi,
): Promise<void> {
  const lastSynced = currentUser.lastSync ?? new Date("2000");
  resetSyncState();
  globalSyncState.stage = SyncStage.Labels;
  const localModelApi = getContainer().require<IModelApi>(IModelApi);
  const localLabelApi = getContainer().require<ILabelApi>(ILabelApi);

  const serverModels = await getAllModels(serverModelApi);
  const localModels = await getAllModels(localModelApi);

  const serverLabels = await serverLabelApi.getLabels(false);
  const localLabels = await localLabelApi.getLabels(false);

  const modifiedServerLabels = forceApplyFieldToObject(
    serverLabels,
    metaFieldExtractor,
  );
  const modifiedLocalLabels = forceApplyFieldToObject(
    localLabels,
    metaFieldExtractor,
  );

  const syncState = computeDifferences(
    modifiedLocalLabels,
    modifiedServerLabels,
    lastSynced,
  );

  function sortFunction(a: Label, b: Label): number {
    if (a.children.some((aChild) => b.meta.id === aChild.id)) {
      // A needs to come after B
      return 1;
    }

    if (b.children.some((bChild) => a.meta.id === bChild.id)) {
      // B needs to come after A
      return -1;
    }

    return 0;
  }

  syncState.toUpload.sort(sortFunction);
  syncState.toDownload.sort(sortFunction);

  await applySyncResult(syncState, {
    upload: (toUpload) =>
      stepUploadToRemote(
        toUpload,
        localLabelApi,
        serverLabelApi,
        localModelApi,
        serverModels,
        false,
      ),
    download: (toDownload) =>
      stepUploadToRemote(
        toDownload,
        serverLabelApi,
        localLabelApi,
        serverModelApi,
        localModels,
        true,
      ),
    syncToServer: (toSync) =>
      stepSyncToRemote(
        toSync,
        localLabelApi,
        serverLabelApi,
        localModelApi,
        serverModelApi,
        serverModels,
        false,
      ),
    syncToLocal: (toSync) =>
      stepSyncToRemote(
        toSync,
        serverLabelApi,
        localLabelApi,
        serverModelApi,
        localModelApi,
        localModels,
        true,
      ),
    deleteServer: (toDelete) => deleteFromRemote(toDelete, serverLabelApi),
    deleteLocal: (toDelete) => deleteFromRemote(toDelete, localLabelApi),
  });
}
