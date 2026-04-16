import type {
  GroupOrderBy,
  Group,
  GroupMeta,
  IGroupApi,
} from "../shared/group_api";
import {
  chunkIdsForCommaSeparatedQuery,
  finalizeChunkedGroupList,
  MODEL_IDS_STR_SAFE_CHUNK_CHARS,
} from "../shared/group_query_chunks";
import type { Model } from "../shared/model_api";
import {
  HttpMethod,
  type IServerRequestApi,
} from "../shared/server_request_api";
import type { Share } from "../shared/share_api";
import { parseRawGroup, type RawGroup } from "../tauri/group";

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
    const endpoint = `/shares/${this.share.id}/groups`;
    const base = {
      group_ids: group_ids,
      label_ids: label_ids,
      order_by: order_by,
      text_search: text_search,
      include_ungrouped_models: include_ungrouped_models,
    };

    const joined =
      model_ids != null && model_ids.length > 0 ? model_ids.join(",") : null;

    const mustChunk =
      joined != null && joined.length > MODEL_IDS_STR_SAFE_CHUNK_CHARS;

    if (!mustChunk) {
      const data = {
        ...base,
        model_ids_str: model_ids?.join(","),
        page: page,
        page_size: page_size,
      };

      const response = await this.requestApi.request<RawGroup[]>(
        endpoint,
        HttpMethod.GET,
        data,
      );
      return response.map((rawGroup) => parseRawGroup(rawGroup));
    }

    const chunks = chunkIdsForCommaSeparatedQuery(
      model_ids!,
      MODEL_IDS_STR_SAFE_CHUNK_CHARS,
    );
    const collected: Group[] = [];

    for (const chunk of chunks) {
      let p = 1;
      while (true) {
        const data = {
          ...base,
          model_ids_str: chunk.join(","),
          page: p,
          page_size: page_size,
        };

        const response = await this.requestApi.request<RawGroup[]>(
          endpoint,
          HttpMethod.GET,
          data,
        );
        const pageGroups = response.map((rawGroup) => parseRawGroup(rawGroup));
        collected.push(...pageGroups);
        if (pageGroups.length < page_size) {
          break;
        }
        p += 1;
      }
    }

    return finalizeChunkedGroupList(collected, order_by);
  }

  async addGroup(_name: string): Promise<GroupMeta> {
    throw new Error("Method not implemented.");
  }

  async editGroup(_group: GroupMeta): Promise<void> {}

  async deleteGroup(_group: GroupMeta): Promise<void> {}

  async addModelsToGroup(
    _group: GroupMeta,
    _models: readonly { id: number }[],
  ): Promise<void> {}

  async removeModelsFromGroup(_models: Model[]): Promise<void> {}

  async getGroupCount(_include_ungrouped_models: boolean): Promise<number> {
    return this.share.modelIds.length;
  }
}
