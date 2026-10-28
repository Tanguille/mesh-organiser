import {
  modelMatchesSearch,
  modelOrderByComparator,
  type IModelApi,
  type Model,
  type ModelFilter,
  type ModelFlags,
} from "../shared/model_api";
import {
  mockGroups,
  mockLabels,
  mockModels,
  modelGroupMap,
  modelLabelsMap,
  resolveLabels,
} from "./mock_data";

const trashedModels = new Map<number, Model>();

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

  async getModelCount(flags: ModelFlags | null): Promise<number> {
    const models = this.filterByFlags(Array.from(mockModels.values()), flags);

    return models.length;
  }

  async deleteModels(models: Model[]): Promise<void> {
    for (const { id } of models) {
      const model = mockModels.get(id);
      if (!model) continue;

      model.deleted = new Date();
      model.group = model.group
        ? (mockGroups.get(model.group.id) ?? null)
        : null;
      model.labels = resolveLabels(model.labels.map((label) => label.id));
      trashedModels.set(model.id, model);
      mockModels.delete(model.id);
      modelGroupMap.delete(model.id);
      modelLabelsMap.delete(model.id);
    }
  }

  async getTrashedModels(): Promise<Model[]> {
    return [...trashedModels.values()].sort(
      (left, right) =>
        (right.deleted?.getTime() ?? 0) - (left.deleted?.getTime() ?? 0),
    );
  }

  async restoreModels(models: Model[]): Promise<void> {
    if (models.some((model) => !trashedModels.has(model.id))) {
      throw new Error("Failed to find model to restore");
    }

    for (const id of new Set(models.map((model) => model.id))) {
      const model = trashedModels.get(id)!;
      model.deleted = null;
      // Demo ids can be reused after deletion; only retain the original associations.
      if (model.group && mockGroups.get(model.group.id) !== model.group) {
        model.group = null;
      }
      model.labels = model.labels.filter(
        (label) => mockLabels.get(label.id) === label,
      );
      mockModels.set(id, model);
      if (model.group) modelGroupMap.set(id, model.group.id);
      if (model.labels.length > 0) {
        modelLabelsMap.set(
          id,
          model.labels.map((label) => label.id),
        );
      }
      trashedModels.delete(id);
    }
  }
}
