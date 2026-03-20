import type { Group } from "../shared/group_api";
import type { IResourceApi, ResourceMeta } from "../shared/resource_api";

export class WebShareResourceApi implements IResourceApi {
  async getResources(): Promise<ResourceMeta[]> {
    return [];
  }

  async addResource(_name: string): Promise<ResourceMeta> {
    throw new Error("Method not implemented.");
  }

  async editResource(_resource: ResourceMeta): Promise<void> {}

  async deleteResource(_resource: ResourceMeta): Promise<void> {}

  async setResourceOnGroup(
    _resource: ResourceMeta | null,
    _group_id: number,
  ): Promise<void> {}

  async getGroupsForResource(_resource: ResourceMeta): Promise<Group[]> {
    return [];
  }
}
