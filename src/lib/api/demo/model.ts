import {
  modelMatchesSearch,
  modelOrderByComparator,
  type IModelApi,
  type Model,
  type ModelFilter,
  type ModelFlags,
} from "../shared/model_api";
import { mockModels, modelGroupMap, modelLabelsMap } from "./mock_data";

export class DemoModelApi implements IModelApi {
  private filterByFlags(models: Model[], flags: ModelFlags | null): Model[] {
    if (!flags) {
      return models;
    }
    return models.filter(
      (m) =>
        m.flags.printed === flags.printed &&
        m.flags.favorite === flags.favorite,
    );
  }

  async getModels(
    filter: ModelFilter,
    page: number,
    page_size: number,
  ): Promise<Model[]> {
    const {
      modelIds: model_ids,
      groupIds: group_ids,
      labelIds: label_ids,
      orderBy: order_by,
      textSearch: text_search,
      fileTypes: file_types,
      flags,
    } = filter;
    let models = Array.from(mockModels.values());

    if (model_ids) {
      models = models.filter((m) => model_ids.includes(m.id));
    }

    if (group_ids) {
      models = models.filter((m) => {
        const groupId = modelGroupMap.get(m.id);
        return groupId && group_ids.includes(groupId);
      });
    }

    if (label_ids) {
      models = models.filter((m) => {
        const modelLabelIds = modelLabelsMap.get(m.id) || [];
        return label_ids.some((lid) => modelLabelIds.includes(lid));
      });
    }

    if (text_search) {
      const searchLower = text_search.toLowerCase();
      models = models.filter((m) => modelMatchesSearch(m, searchLower));
    }

    models = this.filterByFlags(models, flags);

    if (file_types) {
      models = models.filter((m) => file_types.includes(m.blob.filetype));
    }

    models.sort(modelOrderByComparator(order_by));

    const start = (page - 1) * page_size;
    const end = start + page_size;
    return models.slice(start, end);
  }

  async editModel(model: Model): Promise<void> {
    const existingModel = mockModels.get(model.id);
    if (!existingModel) {
      throw new Error(`Model with id ${model.id} not found`);
    }

    existingModel.name = model.name;
    existingModel.link = model.link;
    existingModel.description = model.description;
    existingModel.flags = { ...model.flags };
  }

  async deleteModel(model: Model): Promise<void> {
    mockModels.delete(model.id);

    modelGroupMap.delete(model.id);
    modelLabelsMap.delete(model.id);
  }

  async getModelCount(flags: ModelFlags | null): Promise<number> {
    const models = this.filterByFlags(Array.from(mockModels.values()), flags);

    return models.length;
  }

  async deleteModels(models: Model[]): Promise<void> {
    for (const model of models) {
      await this.deleteModel(model);
    }
  }
}
