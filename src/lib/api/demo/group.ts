import { configuration } from "#lib/configuration.svelte";
import {
  createGroupInstance,
  createGroupMetaInstance,
  groupOrderByComparator,
  type DeletedGroup,
  type Group,
  type GroupFilter,
  type GroupMeta,
  type GroupRestoreRequest,
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
import { groupHasTrashedModels, trashedModels } from "./model";

const trashedGroups = new Map<
  number,
  {
    group: GroupMeta;
    deleted: Date;
    members: Model[];
    deletionOperationId: string;
  }
>();
const lastDeletion = new Map<
  number,
  { groupGlobalId: string; deletionOperationId: string }
>();
let lastIssuedGroupId = 0;
let lastDeletionId = 0;

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
      const matchesGroupName =
        text_search &&
        groupMeta.name.toLowerCase().includes(text_search.toLowerCase());

      const modelsInGroup = collectGroupModels(
        model_ids,
        label_ids,
        matchesGroupName ? null : text_search,
        (modelId) => modelGroupMap.get(modelId) === groupId,
      );

      if (modelsInGroup.length === 0) {
        const hasLiveModels = [...modelGroupMap.values()].includes(groupId);
        const matchesName =
          !text_search ||
          groupMeta.name.toLowerCase().includes(text_search.toLowerCase());
        if (
          model_ids ||
          label_ids ||
          file_types ||
          hasLiveModels ||
          groupHasTrashedModels(groupMeta) ||
          !matchesName
        )
          continue;
      }

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
        !file_types ||
        g.models.some((m) => file_types.includes(m.blob.filetype)),
    );

    filteredGroups.sort(groupOrderByComparator(order_by));

    const start = (page - 1) * page_size;
    const end = start + page_size;
    return filteredGroups.slice(start, end);
  }

  async addGroup(name: string): Promise<GroupMeta> {
    let maxId = lastIssuedGroupId;
    mockGroups.forEach((_group, id) => {
      if (id > maxId) maxId = id;
    });

    lastIssuedGroupId = maxId + 1;
    const newGroup = createGroupMetaInstance(
      lastIssuedGroupId,
      name,
      new Date().toISOString(),
      new Date().toISOString(),
      `demo-group-${lastIssuedGroupId}`,
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
    const existingGroup = mockGroups.get(group.id);
    if (!existingGroup) return;

    if (!existingGroup.uniqueGlobalId)
      existingGroup.uniqueGlobalId = `demo-group-${existingGroup.id}`;
    const deletionOperationId = `demo-deletion-${++lastDeletionId}`;
    lastDeletion.set(group.id, {
      groupGlobalId: existingGroup.uniqueGlobalId,
      deletionOperationId,
    });
    lastIssuedGroupId = Math.max(lastIssuedGroupId, group.id);
    const members: Model[] = [];
    mockGroups.delete(group.id);

    mockModels.forEach((model, modelId) => {
      if (modelGroupMap.get(modelId) === group.id) {
        members.push(model);
        model.group = null;
        modelGroupMap.delete(modelId);
      }
    });
    trashedModels.forEach((model) => {
      if (model.group === existingGroup) {
        members.push(model);
        model.group = null;
      }
    });
    trashedGroups.set(group.id, {
      group: existingGroup,
      deleted: new Date(),
      members,
      deletionOperationId,
    });
  }

  async getTrashedGroups(): Promise<DeletedGroup[]> {
    return [...trashedGroups.values()]
      .filter(
        ({ deleted }) =>
          deleted.getTime() >=
          Math.floor(Date.now() / 1000) * 1000 -
            configuration.trash_retention_days * 86400000,
      )
      .sort(
        (a, b) =>
          b.deleted.getTime() - a.deleted.getTime() || b.group.id - a.group.id,
      )
      .map(({ group, deleted, deletionOperationId }) => ({
        id: group.id,
        name: group.name,
        deleted: new Date(deleted),
        groupGlobalId: group.uniqueGlobalId,
        deletionOperationId,
      }));
  }

  async restoreGroups(groups: GroupRestoreRequest[]): Promise<void> {
    const cutoff =
      Math.floor(Date.now() / 1000) * 1000 -
      configuration.trash_retention_days * 86400000;
    for (const group of groups) {
      const entry = trashedGroups.get(group.id);
      const latest = lastDeletion.get(group.id);
      if (
        !latest ||
        latest.groupGlobalId !== group.groupGlobalId ||
        latest.deletionOperationId !== group.deletionOperationId ||
        (!entry &&
          mockGroups.get(group.id)?.uniqueGlobalId !== group.groupGlobalId) ||
        (entry && entry.deleted.getTime() < cutoff)
      ) {
        throw new Error(`Group with id ${group.id} not found`);
      }
    }

    const ids = new Set(groups.map((group) => group.id));
    for (const id of ids) {
      const entry = trashedGroups.get(id);
      if (!entry) continue;
      mockGroups.set(id, entry.group);
      for (const model of entry.members) {
        if (
          mockModels.get(model.id) === model &&
          !modelGroupMap.has(model.id)
        ) {
          model.group = entry.group;
          modelGroupMap.set(model.id, id);
        } else if (trashedModels.get(model.id) === model && !model.group) {
          model.group = entry.group;
        }
      }
      trashedGroups.delete(id);
    }
  }

  async addModelsToGroup(
    group: GroupMeta,
    models: Pick<Model, "id">[],
  ): Promise<void> {
    const destination = mockGroups.get(group.id);
    if (!destination) {
      throw new Error(`Group with id ${group.id} not found`);
    }
    models.forEach((model) => {
      const existingModel = mockModels.get(model.id);
      if (existingModel) {
        existingModel.group = destination;
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
    const liveGroupIds = new Set(modelGroupMap.values());
    let count = [...mockGroups.values()].filter(
      (group) => liveGroupIds.has(group.id) || !groupHasTrashedModels(group),
    ).length;

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
