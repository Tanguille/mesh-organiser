export interface ResourceMeta {
  id: number;
  name: string;
  type: string;
  url: string;
  added: Date;
}

export function createResourceMetaInstance(
  id: number,
  name: string,
  type: string,
  url: string,
  added: string,
): ResourceMeta {
  return {
    id,
    name,
    type,
    url,
    added: new Date(added),
  };
}

export interface IResourceApi {
  getResources(): Promise<ResourceMeta[]>;
  addResource(name: string, type: string, url: string): Promise<ResourceMeta>;
  editResource(resource: ResourceMeta): Promise<void>;
  deleteResource(resource: ResourceMeta): Promise<void>;
}

export const IResourceApi = Symbol("IResourceApi");