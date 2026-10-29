import { currentUser } from "#lib/configuration.svelte";
import {
  beginSyncStep,
  globalSyncState,
  resetSyncState,
  SyncStage,
  SyncStep,
} from "#lib/sync.svelte";
import { getContainer } from "../dependency_injection";
import { ILabelApi, type Label } from "../shared/label_api";
import { getAllModels, IModelApi, type Model } from "../shared/model_api";
import {
  applySyncResult,
  computeDifferences,
  filterByGlobalIds,
  remoteCounterparts,
  resolveDirection,
  stepDelete,
  withMetaSyncFields,
  type ResourceSet,
} from "./algorithm";

async function stepUploadToRemote(
  toUpload: Label[],
  localApi: ILabelApi,
  remoteApi: ILabelApi,
  localModelApi: IModelApi,
  remoteModelsById: Map<string, Model>,
  isDownload: boolean,
): Promise<void> {
  beginSyncStep(
    isDownload ? SyncStep.Download : SyncStep.Upload,
    toUpload.length,
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
    await remoteApi.setKeywordsOnLabel(newLabel, keywords);

    const localModelsForLabel = await getAllModels(localModelApi, [
      label.meta.id,
    ]);
    await remoteApi.addLabelToModels(
      newLabel,
      remoteCounterparts(localModelsForLabel, remoteModelsById),
    );
    await remoteApi.setChildrenOnLabel(
      newLabel,
      filterByGlobalIds(remoteLabelMetas, label.children),
    );

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
  remoteModelsById: Map<string, Model>,
  isServerToLocal: boolean,
): Promise<void> {
  beginSyncStep(SyncStep.UpdateMetadata, toSync.length);

  // Fetched once; unlike in the upload step it needs no per-loop maintenance,
  // as this step creates no remote labels.
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
    await remoteApi.addLabelToModels(
      remoteLabel.meta,
      remoteCounterparts(localModelsForLabel, remoteModelsById),
    );
    await remoteApi.setChildrenOnLabel(
      remoteLabel.meta,
      filterByGlobalIds(remoteLabelMetas, localLabel.children),
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
  serverModelsById: Map<string, Model>,
  localModelsById: Map<string, Model>,
): Promise<void> {
  const lastSynced = currentUser.lastSync ?? new Date("2000");
  resetSyncState(SyncStage.Labels);
  const localModelApi = getContainer().require<IModelApi>(IModelApi);
  const localLabelApi = getContainer().require<ILabelApi>(ILabelApi);

  // The two label-list fetches are independent; run them concurrently.
  const [serverLabels, localLabels] = await Promise.all([
    serverLabelApi.getLabels(false),
    localLabelApi.getLabels(false),
  ]);

  const syncState = computeDifferences(
    withMetaSyncFields(localLabels),
    withMetaSyncFields(serverLabels),
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
        serverModelsById,
        false,
      ),
    download: (toDownload) =>
      stepUploadToRemote(
        toDownload,
        serverLabelApi,
        localLabelApi,
        serverModelApi,
        localModelsById,
        true,
      ),
    syncToServer: (toSync) =>
      stepSyncToRemote(
        toSync,
        localLabelApi,
        serverLabelApi,
        localModelApi,
        serverModelApi,
        serverModelsById,
        false,
      ),
    syncToLocal: (toSync) =>
      stepSyncToRemote(
        toSync,
        serverLabelApi,
        localLabelApi,
        serverModelApi,
        localModelApi,
        localModelsById,
        true,
      ),
    deleteServer: (toDelete) =>
      stepDelete(toDelete, (label) => serverLabelApi.deleteLabel(label.meta)),
    deleteLocal: (toDelete) =>
      stepDelete(toDelete, (label) => localLabelApi.deleteLabel(label.meta)),
  });
}
