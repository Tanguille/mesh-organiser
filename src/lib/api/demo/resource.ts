import { type IResourceApi, type ResourceMeta } from "../shared/resource_api";
import type { Group } from "../shared/group_api";

export class DemoResourceApi implements IResourceApi {
  async getResources(): Promise<ResourceMeta[]> {
    // No resources in demo
    return [];
  }

  async addResource(_name: string): Promise<ResourceMeta> {
    throw new Error("Demo mode: Cannot add resources");
  }

  async editResource(_resource: ResourceMeta): Promise<void> {
    throw new Error("Demo mode: Cannot edit resources");
  }

  async deleteResource(_resource: ResourceMeta): Promise<void> {
    throw new Error("Demo mode: Cannot delete resources");
  }

  async setResourceOnGroup(
    _resource: ResourceMeta | null,
    _group_id: number,
  ): Promise<void> {
    throw new Error("Demo mode: Cannot set resources on groups");
  }

  async getGroupsForResource(_resource: ResourceMeta): Promise<Group[]> {
    return [];
  }
}
