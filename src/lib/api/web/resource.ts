import { dateToString } from "$lib/utils";
import type { Group } from "../shared/group_api";
import type { IResourceApi, ResourceMeta } from "../shared/resource_api";
import {
  HttpMethod,
  type IServerRequestApi,
} from "../shared/server_request_api";
import { parseRawGroup, type RawGroup } from "../tauri/group";
import {
  convertResourceFlagsToRaw,
  parseRawResourceMeta,
  type RawResourceMeta,
} from "../tauri/resource";

export class WebResourceApi implements IResourceApi {
  private requestApi: IServerRequestApi;

  constructor(requestApi: IServerRequestApi) {
    this.requestApi = requestApi;
  }

  async getResources(): Promise<ResourceMeta[]> {
    const resources = await this.requestApi.request<RawResourceMeta[]>(
      "/resources",
      HttpMethod.GET,
    );
    return resources.map((raw) => parseRawResourceMeta(raw));
  }

  async addResource(name: string): Promise<ResourceMeta> {
    const data = {
      resource_name: name,
    };

    const resource = await this.requestApi.request<RawResourceMeta>(
      "/resources",
      HttpMethod.POST,
      data,
    );
    return parseRawResourceMeta(resource);
  }

  async editResource(
    resource: ResourceMeta,
    editTimestamp?: boolean,
    editGlobalId?: boolean,
  ): Promise<void> {
    const data: Record<string, unknown> = {
      resource_name: resource.name,
      resource_flags: convertResourceFlagsToRaw(resource.flags),
    };

    if (editTimestamp) {
      data.resource_timestamp = dateToString(resource.lastModified);
    }

    if (editGlobalId) {
      data.resource_global_id = resource.uniqueGlobalId;
    }

    await this.requestApi.request<void>(
      `/resources/${resource.id}`,
      HttpMethod.PUT,
      data,
    );
  }

  async deleteResource(resource: ResourceMeta): Promise<void> {
    await this.requestApi.request<void>(
      `/resources/${resource.id}`,
      HttpMethod.DELETE,
    );
  }

  async setResourceOnGroup(
    resource: ResourceMeta | null,
    group_id: number,
  ): Promise<void> {
    const data = {
      resource_id: resource ? resource.id : null,
    };

    await this.requestApi.request<void>(
      `/groups/${group_id}/resource`,
      HttpMethod.PUT,
      data,
    );
  }

  async getGroupsForResource(resource: ResourceMeta): Promise<Group[]> {
    const groups = await this.requestApi.request<RawGroup[]>(
      `/resources/${resource.id}/groups`,
      HttpMethod.GET,
    );

    return groups.map((group) => parseRawGroup(group));
  }
}
