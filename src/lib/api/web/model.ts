import { dateToString } from "$lib/utils";
import type {
  IModelApi,
  Model,
  ModelFilter,
  ModelFlags,
} from "../shared/model_api";
import {
  buildGetModelsQuery,
  convertModelFlagsToRaw,
  parseRawModel,
  type RawModel,
} from "../shared/raw_model";
import {
  HttpMethod,
  type IServerRequestApi,
} from "../shared/server_request_api";

export class WebModelApi implements IModelApi {
  private requestApi: IServerRequestApi;

  constructor(requestApi: IServerRequestApi) {
    this.requestApi = requestApi;
  }

  async getModels(
    filter: ModelFilter,
    page: number,
    pageSize: number,
  ): Promise<Model[]> {
    const data = buildGetModelsQuery(filter, page, pageSize);

    const response = await this.requestApi.request<RawModel[]>(
      "/models",
      HttpMethod.GET,
      data,
    );
    return response.map((rawModel) => parseRawModel(rawModel));
  }

  async editModel(
    model: Model,
    editTimestamp?: boolean,
    editGlobalId?: boolean,
  ): Promise<void> {
    const data: Record<string, unknown> = {
      model_name: model.name,
      model_url: model.link,
      model_description: model.description,
      model_flags: convertModelFlagsToRaw(model.flags),
    };

    if (editTimestamp) {
      data.model_timestamp = dateToString(model.lastModified);
    }

    if (editGlobalId) {
      data.model_global_id = model.uniqueGlobalId;
    }

    await this.requestApi.request<void>(
      `/models/${model.id}`,
      HttpMethod.PUT,
      data,
    );
  }

  async deleteModel(model: Model): Promise<void> {
    await this.requestApi.request<void>(
      `/models/${model.id}`,
      HttpMethod.DELETE,
    );
  }

  async deleteModels(models: Model[]): Promise<void> {
    const data = {
      model_ids: models.map((model) => model.id),
    };

    await this.requestApi.request<void>(`/models`, HttpMethod.DELETE, data);
  }

  async getModelCount(flags: ModelFlags | null): Promise<number> {
    const data = {
      model_flags: convertModelFlagsToRaw(flags),
    };

    return (
      await this.requestApi.request<{ count: number }>(
        "/models/count",
        HttpMethod.GET,
        data,
      )
    ).count;
  }
}
