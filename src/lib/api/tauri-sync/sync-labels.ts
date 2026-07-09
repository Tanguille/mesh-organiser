import { currentUser } from "$lib/configuration.svelte";
import {
  globalSyncState,
  resetSyncState,
  SyncStage,
  SyncStep,
} from "$lib/sync.svelte";
import { getContainer } from "../dependency_injection";
import { ILabelApi, type Label } from "../shared/label_api";
import { getAllModels, IModelApi, type Model } from "../shared/model_api";
import {
  applySyncResult,
  computeDifferences,
  forceApplyFieldToObject,
  metaFieldExtractor,
  resolveDirection,
  stepDelete,
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

  // Index the remote models once so per-label member resolution is
  // O(label size) instead of a full model-library scan per label.
  const remoteModelsById = new Map(
    remoteModels.map((x) => [x.uniqueGlobalId, x]),
  );

  // Fetch the remote labels once instead of per-iteration. `toUpload` is sorted
  // children-first, so appending each label's final meta (after editLabel pushes
  // its local global id to the remote) reproduces what a refetch would return.
  const remoteLabelMetas = (await remoteApi.getLabels(false)).map(
    (x) => x.meta,
  );

  for (const label of toUpload) {
    const newLabel = await remoteApi.addLabel(
      label.meta.name,
      label.meta.color,
    );

    const keywords = await localApi.getKeywordsForLabel(label.meta);
    remoteApi.setKeywordsOnLabel(newLabel, keywords);

    const localModelsForLabel = await getAllModels(localModelApi, [
      label.meta.id,
    ]);
    const relatedRemoteModels = localModelsForLabel.flatMap(
      (model) => remoteModelsById.get(model.uniqueGlobalId) ?? [],
    );
    await remoteApi.addLabelToModels(newLabel, relatedRemoteModels);

    const relatedRemoteChildLabels = remoteLabelMetas.filter((x) =>
      label.children.some((y) => y.uniqueGlobalId === x.uniqueGlobalId),
    );
    await remoteApi.setChildrenOnLabel(newLabel, relatedRemoteChildLabels);

    label.meta.id = newLabel.id;
    await remoteApi.editLabel(label.meta, true, true);
    remoteLabelMetas.push({ ...label.meta });
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

  // Index the remote models once so per-label member resolution is
  // O(label size) instead of a full model-library scan per label.
  const remoteModelsById = new Map(
    remoteModels.map((x) => [x.uniqueGlobalId, x]),
  );

  // Fetch the remote labels once instead of per-iteration. Unlike the upload
  // step no per-loop maintenance is needed: this step creates no remote labels
  // and pairs are matched by uniqueGlobalId, so the list never changes mid-loop.
  const remoteLabelMetas = (await remoteApi.getLabels(false)).map(
    (x) => x.meta,
  );

  for (const labelSet of toSync) {
    const { remote: remoteLabel, local: localLabel } = resolveDirection(
      labelSet,
      isServerToLocal,
    );

    const keywords = await localApi.getKeywordsForLabel(localLabel.meta);
    await remoteApi.setKeywordsOnLabel(remoteLabel.meta, keywords);

    const [localModelsForLabel, remoteModelsForLabel] = await Promise.all([
      getAllModels(localModelApi, [localLabel.meta.id]),
      getAllModels(remoteModelApi, [remoteLabel.meta.id]),
    ]);
    await remoteApi.removeLabelFromModels(
      remoteLabel.meta,
      remoteModelsForLabel,
    );
    const relatedRemoteModels = localModelsForLabel.flatMap(
      (model) => remoteModelsById.get(model.uniqueGlobalId) ?? [],
    );
    await remoteApi.addLabelToModels(remoteLabel.meta, relatedRemoteModels);

    const relatedRemoteChildLabels = remoteLabelMetas.filter((x) =>
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

export async function syncLabels(
  serverModelApi: IModelApi,
  serverLabelApi: ILabelApi,
  serverModels: Model[],
  localModels: Model[],
): Promise<void> {
  const lastSynced = currentUser.lastSync ?? new Date("2000");
  resetSyncState();
  globalSyncState.stage = SyncStage.Labels;
  const localModelApi = getContainer().require<IModelApi>(IModelApi);
  const localLabelApi = getContainer().require<ILabelApi>(ILabelApi);

  // The two label-list fetches are independent; run them concurrently.
  const [serverLabels, localLabels] = await Promise.all([
    serverLabelApi.getLabels(false),
    localLabelApi.getLabels(false),
  ]);

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
    deleteServer: (toDelete) =>
      stepDelete(toDelete, (label) => serverLabelApi.deleteLabel(label.meta)),
    deleteLocal: (toDelete) =>
      stepDelete(toDelete, (label) => localLabelApi.deleteLabel(label.meta)),
  });
}
