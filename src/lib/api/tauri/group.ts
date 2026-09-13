import { invoke } from "@tauri-apps/api/core";
import {
  type Group,
  type GroupFilter,
  type GroupMeta,
  type IGroupApi,
} from "../shared/group_api";
import { type Model } from "../shared/model_api";
import {
  parseRawGroup,
  parseRawGroupMeta,
  type RawGroup,
  type RawGroupMeta,
} from "../shared/raw_model";
import { dateToString } from "$lib/utils";

export class GroupApi implements IGroupApi {
  async getGroups(
    filter: GroupFilter,
    page: number,
    pageSize: number,
  ): Promise<Group[]> {
    const groups = await invoke<RawGroup[]>("get_groups", {
      modelIds: filter.modelIds,
      groupIds: filter.groupIds,
      labelIds: filter.labelIds,
      orderBy: filter.orderBy,
      textSearch: filter.textSearch,
      fileTypes: filter.fileTypes,
      page,
      pageSize,
      includeUngroupedModels: filter.includeUngroupedModels,
    });

    return groups.map((group) => parseRawGroup(group));
  }

  async addGroup(name: string): Promise<GroupMeta> {
    const group = await invoke<RawGroupMeta>("add_group", { groupName: name });
    return parseRawGroupMeta(group);
  }

  async editGroup(
    group: GroupMeta,
    editTimestamp?: boolean,
    editGlobalId?: boolean,
  ): Promise<void> {
    const data: Record<string, unknown> = {
      groupId: group.id,
      groupName: group.name,
    };

    if (editTimestamp) {
      data.groupTimestamp = dateToString(group.lastModified);
    }

    if (editGlobalId) {
      data.groupGlobalId = group.uniqueGlobalId;
    }

    return await invoke("edit_group", data);
  }

  async deleteGroup(group: GroupMeta): Promise<void> {
    return await invoke("ungroup", { groupId: group.id });
  }

  async addModelsToGroup(
    group: GroupMeta,
    models: Pick<Model, "id">[],
  ): Promise<void> {
    return await invoke("add_models_to_group", {
      groupId: group.id,
      modelIds: models.map((model) => model.id),
    });
  }

  async removeModelsFromGroup(models: Pick<Model, "id">[]): Promise<void> {
    return await invoke("remove_models_from_group", {
      modelIds: models.map((model) => model.id),
    });
  }

  async getGroupCount(include_ungrouped_models: boolean): Promise<number> {
    return await invoke("get_group_count", {
      includeUngroupedModels: include_ungrouped_models,
    });
  }
}
