import {
  createGroupInstance,
  createGroupMetaInstance,
  groupOrderByComparator,
  type Group,
  type GroupFilter,
  type GroupMeta,
  type IGroupApi,
} from "../shared/group_api";
import { modelMatchesSearch, type Model } from "../shared/model_api";
import { convertModelFlagsToRaw } from "../shared/raw_model";
import {
  mockGroups,
  mockModels,
  modelGroupMap,
  modelLabelsMap,
  resolveLabels,
} from "./mock_data";

// The mock models that satisfy the membership predicate and pass the
// model_ids / label_ids / text_search filters. Shared by the grouped and
// ungrouped collection loops, which differ only in the membership predicate.
function collectGroupModels(
  model_ids: number[] | null,
  label_ids: number[] | null,
  text_search: string | null,
  predicate: (modelId: number) => boolean,
): Model[] {
  const searchLower = text_search?.toLowerCase();

  return [...mockModels.values()].filter((model) => {
    const modelLabelIds = modelLabelsMap.get(model.id) || [];

    return (
      predicate(model.id) &&
      (!model_ids || model_ids.includes(model.id)) &&
      (!label_ids ||
        label_ids.some((labelId) => modelLabelIds.includes(labelId))) &&
      (!searchLower || modelMatchesSearch(model, searchLower))
    );
  });
}

export class DemoGroupApi implements IGroupApi {
  async getGroups(
    filter: GroupFilter,
    page: number,
    page_size: number,
  ): Promise<Group[]> {
    const {
      modelIds: model_ids,
      groupIds: group_ids,
      labelIds: label_ids,
      orderBy: order_by,
      textSearch: text_search,
      fileTypes: file_types,
      includeUngroupedModels: include_ungrouped_models,
    } = filter;
    const groups: Group[] = [];

    // The Set drops repeated group ids.
    for (const groupId of new Set(group_ids ?? mockGroups.keys())) {
      const groupMeta = mockGroups.get(groupId);
      if (!groupMeta) continue;

      const modelsInGroup = collectGroupModels(
        model_ids,
        label_ids,
        text_search,
        (modelId) => modelGroupMap.get(modelId) === groupId,
      );

      if (modelsInGroup.length === 0 && !include_ungrouped_models) continue;

      // Union of the members' labels and Printed/Favorite flags, deduped.
      const memberLabelIds = modelsInGroup.flatMap(
        (model) => modelLabelsMap.get(model.id) || [],
      );
      const flags = modelsInGroup.flatMap(
        (model) => convertModelFlagsToRaw(model.flags) ?? [],
      );

      groups.push(
        createGroupInstance(
          groupMeta,
          modelsInGroup,
          resolveLabels([...new Set(memberLabelIds)]),
          null, // No resource in demo
          [...new Set(flags)],
        ),
      );
    }

    if (include_ungrouped_models) {
      const ungroupedModels = collectGroupModels(
        model_ids,
        label_ids,
        text_search,
        (modelId) => !modelGroupMap.has(modelId),
      );

      ungroupedModels.forEach((model) => {
        const modelLabelIds = modelLabelsMap.get(model.id) || [];
        const labels = resolveLabels(modelLabelIds);

        const group = createGroupInstance(
          createGroupMetaInstance(
            model.id * -1,
            model.name,
            model.added.toISOString(),
            model.lastModified.toISOString(),
            "",
          ),
          [model],
          labels,
          null,
          convertModelFlagsToRaw(model.flags) ?? [],
        );

        groups.push(group);
      });
    }

    // Like the SQL backend: keep whole groups that contain any matching type.
    const filteredGroups = groups.filter(
      (g) =>
        g.models.length > 0 &&
        (!file_types ||
          g.models.some((m) => file_types.includes(m.blob.filetype))),
    );

    filteredGroups.sort(groupOrderByComparator(order_by));

    const start = (page - 1) * page_size;
    const end = start + page_size;
    return filteredGroups.slice(start, end);
  }

  async addGroup(name: string): Promise<GroupMeta> {
    let maxId = 0;
    mockGroups.forEach((_group, id) => {
      if (id > maxId) maxId = id;
    });

    const newGroup = createGroupMetaInstance(
      maxId + 1,
      name,
      new Date().toISOString(),
      new Date().toISOString(),
      "",
    );

    mockGroups.set(newGroup.id, newGroup);
    return newGroup;
  }

  async editGroup(group: GroupMeta): Promise<void> {
    const existingGroup = mockGroups.get(group.id);
    if (!existingGroup) {
      throw new Error(`Group with id ${group.id} not found`);
    }

    existingGroup.name = group.name;
  }

  async deleteGroup(group: GroupMeta): Promise<void> {
    mockGroups.delete(group.id);

    mockModels.forEach((model, modelId) => {
      if (modelGroupMap.get(modelId) === group.id) {
        model.group = null;
        modelGroupMap.delete(modelId);
      }
    });
  }

  async addModelsToGroup(
    group: GroupMeta,
    models: Pick<Model, "id">[],
  ): Promise<void> {
    models.forEach((model) => {
      const existingModel = mockModels.get(model.id);
      if (existingModel) {
        existingModel.group = group;
        modelGroupMap.set(model.id, group.id);
      }
    });
  }

  async removeModelsFromGroup(models: Pick<Model, "id">[]): Promise<void> {
    models.forEach((model) => {
      const existingModel = mockModels.get(model.id);
      if (existingModel) {
        existingModel.group = null;
        modelGroupMap.delete(model.id);
      }
    });
  }

  async getGroupCount(include_ungrouped_models: boolean): Promise<number> {
    let count = mockGroups.size;

    if (include_ungrouped_models) {
      let ungroupedCount = 0;
      mockModels.forEach((_model, modelId) => {
        if (!modelGroupMap.has(modelId)) {
          ungroupedCount++;
        }
      });
      count += ungroupedCount;
    }

    return count;
  }
}
