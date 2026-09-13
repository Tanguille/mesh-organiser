import { dateToString } from "$lib/utils";
import {
  buildGetGroupsQuery,
  type Group,
  type GroupFilter,
  type GroupMeta,
  type IGroupApi,
} from "../shared/group_api";
import type { Model } from "../shared/model_api";
import {
  HttpMethod,
  type IServerRequestApi,
} from "../shared/server_request_api";
import {
  parseRawGroup,
  parseRawGroupMeta,
  type RawGroup,
  type RawGroupMeta,
} from "../shared/raw_model";

export class WebGroupApi implements IGroupApi {
  private requestApi: IServerRequestApi;

  constructor(requestApi: IServerRequestApi) {
    this.requestApi = requestApi;
  }

  async getGroups(
    filter: GroupFilter,
    page: number,
    pageSize: number,
  ): Promise<Group[]> {
    const data = buildGetGroupsQuery(filter, page, pageSize);

    const response = await this.requestApi.request<RawGroup[]>(
      "/groups",
      HttpMethod.GET,
      data,
    );
    return response.map((rawGroup) => parseRawGroup(rawGroup));
  }

  async addGroup(name: string): Promise<GroupMeta> {
    const data = {
      group_name: name,
    };

    const response = await this.requestApi.request<RawGroupMeta>(
      "/groups",
      HttpMethod.POST,
      data,
    );
    return parseRawGroupMeta(response);
  }

  async editGroup(
    group: GroupMeta,
    editTimestamp?: boolean,
    editGlobalId?: boolean,
  ): Promise<void> {
    const data: Record<string, unknown> = {
      group_name: group.name,
    };

    if (editTimestamp) {
      data.group_timestamp = dateToString(group.lastModified);
    }

    if (editGlobalId) {
      data.group_global_id = group.uniqueGlobalId;
    }

    await this.requestApi.request<void>(
      `/groups/${group.id}`,
      HttpMethod.PUT,
      data,
    );
  }

  async deleteGroup(group: GroupMeta): Promise<void> {
    await this.requestApi.request<void>(
      `/groups/${group.id}`,
      HttpMethod.DELETE,
    );
  }

  async addModelsToGroup(
    group: GroupMeta,
    models: Pick<Model, "id">[],
  ): Promise<void> {
    const data = {
      model_ids: models.map((model) => model.id),
    };

    await this.requestApi.request<void>(
      `/groups/${group.id}/models`,
      HttpMethod.POST,
      data,
    );
  }

  async removeModelsFromGroup(models: Pick<Model, "id">[]): Promise<void> {
    const data = {
      model_ids: models.map((model) => model.id),
    };

    await this.requestApi.request<void>(
      `/groups/detach_models`,
      HttpMethod.DELETE,
      data,
    );
  }

  async getGroupCount(include_ungrouped_models: boolean): Promise<number> {
    const data = {
      include_ungrouped_models: include_ungrouped_models,
    };

    return (
      await this.requestApi.request<{ count: number }>(
        "/groups/count",
        HttpMethod.GET,
        data,
      )
    ).count;
  }
}
