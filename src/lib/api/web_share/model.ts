import type {
  IModelApi,
  ModelFilter,
  ModelFlags,
  Model,
} from "../shared/model_api";
import {
  buildGetModelsQuery,
  parseRawModel,
  type RawModel,
} from "../shared/raw_model";
import {
  HttpMethod,
  type IServerRequestApi,
} from "../shared/server_request_api";
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
    const data = buildGetModelsQuery(filter, page, pageSize);

    const response = await this.requestApi.request<RawModel[]>(
      `/shares/${this.share.id}/models`,
      HttpMethod.GET,
      data,
    );
    return response.map((rawModel) => parseRawModel(rawModel));
  }

  async editModel(_model: Model): Promise<void> {}

  async deleteModel(_model: Model): Promise<void> {}

  async deleteModels(_models: Model[]): Promise<void> {}

  async getModelCount(_flags: ModelFlags | null): Promise<number> {
    return this.share.modelIds.length;
  }
}
