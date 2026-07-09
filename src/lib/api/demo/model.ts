import {
  modelMatchesSearch,
  modelOrderByComparator,
  type IModelApi,
  type Model,
  type ModelFlags,
  type ModelOrderBy,
} from "../shared/model_api";
import { mockModels, modelGroupMap, modelLabelsMap } from "./mock_data";

export class DemoModelApi implements IModelApi {
  private filterByFlags(models: Model[], flags: ModelFlags | null): Model[] {
    if (flags) {
      if (flags.printed !== undefined) {
        models = models.filter((m) => m.flags.printed === flags.printed);
      }
      if (flags.favorite !== undefined) {
        models = models.filter((m) => m.flags.favorite === flags.favorite);
      }
    }

    return models;
  }

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
    let models = Array.from(mockModels.values());

    // Filter by model IDs
    if (model_ids) {
      models = models.filter((m) => model_ids.includes(m.id));
    }

    // Filter by group IDs
    if (group_ids) {
      models = models.filter((m) => {
        const groupId = modelGroupMap.get(m.id);
        return groupId && group_ids.includes(groupId);
      });
    }

    // Filter by label IDs
    if (label_ids) {
      models = models.filter((m) => {
        const modelLabelIds = modelLabelsMap.get(m.id) || [];
        return label_ids.some((lid) => modelLabelIds.includes(lid));
      });
    }

    // Filter by text search
    if (text_search) {
      const searchLower = text_search.toLowerCase();
      models = models.filter((m) => modelMatchesSearch(m, searchLower));
    }

    // Filter by flags
    models = this.filterByFlags(models, flags);

    // Sort models
    models.sort(modelOrderByComparator(order_by));

    // Apply pagination
    const start = (page - 1) * page_size;
    const end = start + page_size;
    return models.slice(start, end);
  }

  async editModel(model: Model): Promise<void> {
    // Update the model in the mock data
    const existingModel = mockModels.get(model.id);
    if (!existingModel) {
      throw new Error(`Model with id ${model.id} not found`);
    }

    // Update mutable properties
    existingModel.name = model.name;
    existingModel.link = model.link;
    existingModel.description = model.description;
    existingModel.flags = { ...model.flags };
  }

  async deleteModel(model: Model): Promise<void> {
    // Remove model from mock data
    mockModels.delete(model.id);

    // Clean up relationships
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
