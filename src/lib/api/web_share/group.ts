import type {
  GroupFilter,
  Group,
  GroupMeta,
  IGroupApi,
} from "../shared/group_api";
import type { Model } from "../shared/model_api";
import { fetchGroupPage } from "../shared/raw_model";
import type { IServerRequestApi } from "../shared/server_request_api";
import type { Share } from "../shared/share_api";

export class WebShareGroupApi implements IGroupApi {
  private requestApi: IServerRequestApi;
  private share: Share;

  constructor(requestApi: IServerRequestApi, share: Share) {
    this.requestApi = requestApi;
    this.share = share;
  }

  async getGroups(
    filter: GroupFilter,
    page: number,
    pageSize: number,
  ): Promise<Group[]> {
    return fetchGroupPage(
      this.requestApi,
      `/shares/${this.share.id}/groups`,
      filter,
      page,
      pageSize,
    );
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
