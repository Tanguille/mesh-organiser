import { invoke } from "@tauri-apps/api/core";
import {
  type Model,
  type IModelApi,
  type ModelFilter,
  type ModelFlags,
} from "../shared/model_api";
import {
  type RawModel,
  convertModelFlagsToRaw,
  parseRawModel,
} from "../shared/raw_model";
import { dateToString } from "$lib/utils";

export class ModelApi implements IModelApi {
  async getModels(
    filter: ModelFilter,
    page: number,
    pageSize: number,
  ): Promise<Model[]> {
    const models = await invoke<RawModel[]>("get_models", {
      modelIds: filter.modelIds,
      groupIds: filter.groupIds,
      labelIds: filter.labelIds,
      orderBy: filter.orderBy,
      textSearch: filter.textSearch,
      modelFlags: convertModelFlagsToRaw(filter.flags),
      fileTypes: filter.fileTypes,
      page,
      pageSize,
    });

    return models.map((model) => parseRawModel(model));
  }

  async editModel(
    model: Model,
    editTimestamp?: boolean,
    editGlobalId?: boolean,
  ): Promise<void> {
    const data: Record<string, unknown> = {
      modelId: model.id,
      modelName: model.name,
      modelUrl: model.link,
      modelDescription: model.description,
      modelFlags: convertModelFlagsToRaw(model.flags),
    };

    if (editTimestamp) {
      data.modelTimestamp = dateToString(model.lastModified);
    }

    if (editGlobalId) {
      data.modelGlobalId = model.uniqueGlobalId;
    }

    await invoke("edit_model", data);
  }

  async deleteModel(model: Model): Promise<void> {
    await invoke("delete_model", { modelId: model.id });
  }

  async deleteModels(models: Model[]): Promise<void> {
    await invoke("delete_models", { modelIds: models.map((x) => x.id) });
  }

  async getModelCount(flags: ModelFlags | null): Promise<number> {
    return await invoke("get_model_count", {
      flags: convertModelFlagsToRaw(flags),
    });
  }
}
