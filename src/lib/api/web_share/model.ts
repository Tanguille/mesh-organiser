import type {
  IModelApi,
  ModelFilter,
  ModelFlags,
  Model,
} from "../shared/model_api";
import { fetchModelPage } from "../shared/raw_model";
import type { IServerRequestApi } from "../shared/server_request_api";
import type { Share } from "../shared/share_api";

export class WebShareModelApi implements IModelApi {
  private requestApi: IServerRequestApi;
  private share: Share;

  constructor(requestApi: IServerRequestApi, share: Share) {
    this.requestApi = requestApi;
    this.share = share;
  }

  async getModels(
    filter: ModelFilter,
    page: number,
    pageSize: number,
  ): Promise<Model[]> {
    return fetchModelPage(
      this.requestApi,
      `/shares/${this.share.id}/models`,
      filter,
      page,
      pageSize,
    );
  }

  async editModel(_model: Model): Promise<void> {}

  async deleteModel(_model: Model): Promise<void> {}

  async deleteModels(_models: Model[]): Promise<void> {}

  async getModelCount(_flags: ModelFlags | null): Promise<number> {
    return this.share.modelIds.length;
  }
}
