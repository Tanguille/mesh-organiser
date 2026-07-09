import {
  buildGetGroupsQuery,
  type GroupOrderBy,
  type Group,
  type GroupMeta,
  type IGroupApi,
} from "../shared/group_api";
import type { Model } from "../shared/model_api";
import { parseRawGroup, type RawGroup } from "../shared/raw_model";
import {
  HttpMethod,
  type IServerRequestApi,
} from "../shared/server_request_api";
import type { Share } from "../shared/share_api";

export class WebShareGroupApi implements IGroupApi {
  private requestApi: IServerRequestApi;
  private share: Share;

  constructor(requestApi: IServerRequestApi, share: Share) {
    this.requestApi = requestApi;
    this.share = share;
  }

  async getGroups(
    model_ids: number[] | null,
    group_ids: number[] | null,
    label_ids: number[] | null,
    order_by: GroupOrderBy,
    text_search: string | null,
    page: number,
    page_size: number,
    include_ungrouped_models: boolean,
  ): Promise<Group[]> {
    const data = buildGetGroupsQuery(
      model_ids,
      group_ids,
      label_ids,
      order_by,
      text_search,
      page,
      page_size,
      include_ungrouped_models,
    );

    const response = await this.requestApi.request<RawGroup[]>(
      `/shares/${this.share.id}/groups`,
      HttpMethod.GET,
      data,
    );
    return response.map((rawGroup) => parseRawGroup(rawGroup));
  }

  async addGroup(_name: string): Promise<GroupMeta> {
    throw new Error("Method not implemented.");
  }

  async editGroup(_group: GroupMeta): Promise<void> {}

  async deleteGroup(_group: GroupMeta): Promise<void> {}

  async addModelsToGroup(
    _group: GroupMeta,
    _models: Pick<Model, "id">[],
  ): Promise<void> {}

  async removeModelsFromGroup(_models: Pick<Model, "id">[]): Promise<void> {}

  async getGroupCount(_include_ungrouped_models: boolean): Promise<number> {
    return this.share.modelIds.length;
  }
}
