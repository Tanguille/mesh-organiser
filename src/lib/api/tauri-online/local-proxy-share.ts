import { getAllModels, type IModelApi, type Model } from "../shared/model_api";
import type { IServerRequestApi } from "../shared/server_request_api";
import type { Share } from "../shared/share_api";
import { WebShareApi } from "../web/share";

export class TauriProxyShareApi extends WebShareApi {
  private remoteModelApi: IModelApi;
  private localModelApi: IModelApi;

  constructor(
    requestApi: IServerRequestApi,
    remoteModelApi: IModelApi,
    localModelApi: IModelApi,
  ) {
    super(requestApi);
    this.remoteModelApi = remoteModelApi;
    this.localModelApi = localModelApi;
  }

  // Fetches the full remote model list and resolves the given local models to
  // their remote equivalents by uniqueGlobalId, throwing if any are missing.
  private async mapToRemoteModels(models: Model[]): Promise<Model[]> {
    const allRemoteModels = await getAllModels(this.remoteModelApi);
    const selectedGlobalIds = new Set(
      models.map((model) => model.uniqueGlobalId),
    );
    const remoteModels = allRemoteModels.filter((model) =>
      selectedGlobalIds.has(model.uniqueGlobalId),
    );

    if (remoteModels.length !== models.length) {
      throw new Error(
        "Some models to set on the share do not exist on the remote server",
      );
    }

    return remoteModels;
  }

  async getShares(): Promise<Share[]> {
    const shares = await super.getShares();
    if (shares.length === 0) return shares;

    const [localModels, remoteModels] = await Promise.all([
      getAllModels(this.localModelApi),
      getAllModels(this.remoteModelApi),
    ]);

    const globalIdByRemoteId = new Map<number, string>();
    for (const model of remoteModels) {
      // A repeated remote id resolves to the first model in catalog order.
      if (!globalIdByRemoteId.has(model.id))
        globalIdByRemoteId.set(model.id, model.uniqueGlobalId);
    }

    for (const share of shares) {
      const remoteGlobalIds = share.modelIds
        .map((id) => globalIdByRemoteId.get(id))
        .filter((id) => id !== undefined);

      if (remoteGlobalIds.length !== share.modelIds.length) {
        console.error(
          `Some models in share ${share.id} do not exist on the remote server`,
        );
      }

      const memberGlobalIds = new Set(remoteGlobalIds);
      const localModelIds = localModels
        .filter((m) => memberGlobalIds.has(m.uniqueGlobalId))
        .map((m) => m.id);

      if (localModelIds.length !== remoteGlobalIds.length) {
        console.error(
          `Some models in share ${share.id} do not exist on the local server`,
        );
      }

      share.modelIds = localModelIds;
    }

    return shares;
  }

  async setModelsOnShare(share: Share, models: Model[]): Promise<void> {
    const remoteModels = await this.mapToRemoteModels(models);
    return super.setModelsOnShare(share, remoteModels);
  }
}
