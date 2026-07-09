import { invoke } from "@tauri-apps/api/core";
import {
  type Model,
  type IModelApi,
  type ModelFlags,
  type ModelOrderBy,
} from "../shared/model_api";
import {
  type RawModel,
  convertModelFlagsToRaw,
  parseRawModel,
} from "../shared/raw_model";
import { dateToString } from "$lib/utils";

export class ModelApi implements IModelApi {
  async getModels(
    model_ids: number[] | null,
    group_ids: number[] | null,
    label_ids: number[] | null,
    order_by: ModelOrderBy,
    text_search: string | null,
    page: number,
    page_size: number,
    flags: ModelFlags | null,
  ): Promise<Model[]> {
    const models = await invoke<RawModel[]>("get_models", {
      modelIds: model_ids,
      groupIds: group_ids,
      labelIds: label_ids,
      orderBy: order_by,
      textSearch: text_search,
      modelFlags: convertModelFlagsToRaw(flags),
      page: page,
      pageSize: page_size,
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
