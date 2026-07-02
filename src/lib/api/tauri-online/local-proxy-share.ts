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
  private async mapToRemoteModels(
    models: Model[],
    context: string,
  ): Promise<Model[]> {
    const allRemoteModels = await getAllModels(this.remoteModelApi);
    const remoteModels = allRemoteModels.filter((remoteModel) =>
      models.some(
        (localModel) =>
          localModel.uniqueGlobalId === remoteModel.uniqueGlobalId,
      ),
    );

    if (remoteModels.length !== models.length) {
      throw new Error(
        `Some models to ${context} do not exist on the remote server`,
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

    for (const share of shares) {
      const remoteGlobalIds = share.modelIds
        .map((id) => remoteModels.find((m) => m.id === id)?.uniqueGlobalId)
        .filter((id) => id !== undefined) as string[];

      if (remoteGlobalIds.length !== share.modelIds.length) {
        console.error(
          `Some models in share ${share.id} do not exist on the remote server`,
        );
      }

      const localModelIds = localModels
        .filter((m) => remoteGlobalIds.includes(m.uniqueGlobalId))
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

  async addModelsToShare(share: Share, models: Model[]): Promise<void> {
    const remoteModels = await this.mapToRemoteModels(
      models,
      "add to the share",
    );
    return super.addModelsToShare(share, remoteModels);
  }

  async setModelsOnShare(share: Share, models: Model[]): Promise<void> {
    const remoteModels = await this.mapToRemoteModels(
      models,
      "set on the share",
    );
    return super.setModelsOnShare(share, remoteModels);
  }
}
