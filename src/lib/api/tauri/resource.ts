import { invoke } from "@tauri-apps/api/core";
import { type ResourceMeta, type IResourceApi } from "../shared/resource_api";
import type { Group } from "../shared/group_api";
import {
  convertResourceFlagsToRaw,
  parseRawGroup,
  parseRawResourceMeta,
  type RawGroup,
  type RawResourceMeta,
} from "../shared/raw_model";
import { dateToString } from "$lib/utils";

export class ResourceApi implements IResourceApi {
  async getResources(): Promise<ResourceMeta[]> {
    const raw = await invoke<RawResourceMeta[]>("get_resources");
    return raw.map((resource) => parseRawResourceMeta(resource));
  }

  async addResource(name: string): Promise<ResourceMeta> {
    const resource = await invoke<RawResourceMeta>("add_resource", {
      resourceName: name,
    });
    return parseRawResourceMeta(resource);
  }

  async editResource(
    resource: ResourceMeta,
    editTimestamp?: boolean,
    editGlobalId?: boolean,
  ): Promise<void> {
    const data: Record<string, unknown> = {
      resourceId: resource.id,
      resourceName: resource.name,
      resourceFlags: convertResourceFlagsToRaw(resource.flags),
    };

    if (editTimestamp) {
      data.resourceTimestamp = dateToString(resource.lastModified);
    }

    if (editGlobalId) {
      data.resourceGlobalId = resource.uniqueGlobalId;
    }

    return await invoke("edit_resource", data);
  }

  async deleteResource(resource: ResourceMeta): Promise<void> {
    return await invoke("remove_resource", { resourceId: resource.id });
  }

  async setResourceOnGroup(
    resource: ResourceMeta | null,
    group_id: number,
  ): Promise<void> {
    return await invoke("set_resource_on_group", {
      resourceId: resource ? resource.id : null,
      groupId: group_id,
    });
  }

  async getGroupsForResource(resource: ResourceMeta): Promise<Group[]> {
    const groups = await invoke<RawGroup[]>("get_groups_for_resource", {
      resourceId: resource.id,
    });
    return groups.map((group) => parseRawGroup(group));
  }
}
