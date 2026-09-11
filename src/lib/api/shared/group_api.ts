import { normalizeFileTypeFilter, type FileType } from "./blob_api";
import type { LabelMeta } from "./label_api";
import {
  MAX_PAGE_SIZE,
  modelMatchesSearch,
  stringArrayToModelFlags,
  type Model,
  type ModelFlags,
} from "./model_api";
import type { ResourceMeta } from "./resource_api";
import {
  GeneratorStreamManager,
  pagedStream,
  type StreamFilter,
} from "./stream_manager";

export interface GroupMeta {
  id: number;
  name: string;
  created: Date;
  lastModified: Date;
  uniqueGlobalId: string;
}

export function createGroupMetaInstance(
  id: number,
  name: string,
  created: string,
  lastModified: string,
  uniqueGlobalId: string,
): GroupMeta {
  return {
    id,
    name,
    created: new Date(created),
    lastModified: new Date(lastModified),
    uniqueGlobalId: uniqueGlobalId,
  };
}

export interface Group {
  meta: GroupMeta;
  models: Model[];
  labels: LabelMeta[];
  resource: ResourceMeta | null;
  flags: ModelFlags;
}

export function createGroupInstance(
  meta: GroupMeta,
  models: Model[],
  labels: LabelMeta[],
  resource: ResourceMeta | null,
  flags: string[],
): Group {
  if (meta.id >= 0) {
    models.forEach((model) => (model.group = meta));
  }

  return {
    meta,
    models,
    labels,
    resource,
    flags: stringArrayToModelFlags(flags),
  };
}

export enum GroupOrderBy {
  CreatedAsc = "CreatedAsc",
  CreatedDesc = "CreatedDesc",
  NameAsc = "NameAsc",
  NameDesc = "NameDesc",
  ModifiedAsc = "ModifiedAsc",
  ModifiedDesc = "ModifiedDesc",
}

// Single home for the in-memory GroupOrderBy semantics, shared by the demo
// API and the predefined stream manager so the copies cannot drift.
export function groupOrderByComparator(
  orderBy: GroupOrderBy,
): (a: Group, b: Group) => number {
  return (a, b) => {
    switch (orderBy) {
      case GroupOrderBy.CreatedAsc:
        return a.meta.created.getTime() - b.meta.created.getTime();
      case GroupOrderBy.CreatedDesc:
        return b.meta.created.getTime() - a.meta.created.getTime();
      case GroupOrderBy.NameAsc:
        return a.meta.name.localeCompare(b.meta.name);
      case GroupOrderBy.NameDesc:
        return b.meta.name.localeCompare(a.meta.name);
      case GroupOrderBy.ModifiedAsc:
        return a.meta.lastModified.getTime() - b.meta.lastModified.getTime();
      case GroupOrderBy.ModifiedDesc:
        return b.meta.lastModified.getTime() - a.meta.lastModified.getTime();
      default:
        return 0;
    }
  };
}

// Builds the shared getGroups request body used by both the web and
// web-share group endpoints (they differ only in the endpoint path). The
// model_ids_str field is a hack to bypass the request uri becoming too large.
export function buildGetGroupsQuery(
  filter: GroupFilter,
  page: number,
  page_size: number,
) {
  return {
    // Hack to bypass request uri becoming too large
    model_ids_str: filter.modelIds?.join(","),
    group_ids: filter.groupIds,
    label_ids: filter.labelIds,
    order_by: filter.orderBy,
    text_search: filter.textSearch,
    file_types: filter.fileTypes,
    page,
    page_size,
    include_ungrouped_models: filter.includeUngroupedModels,
  };
}

export interface GroupFilter extends StreamFilter<GroupOrderBy> {
  modelIds: number[] | null;
  groupIds: number[] | null;
  labelIds: number[] | null;
  includeUngroupedModels: boolean;
}

export function defaultGroupFilter(
  overrides: Partial<GroupFilter> = {},
): GroupFilter {
  return {
    modelIds: null,
    groupIds: null,
    labelIds: null,
    orderBy: GroupOrderBy.ModifiedDesc,
    textSearch: null,
    fileTypes: null,
    includeUngroupedModels: false,
    ...overrides,
  };
}

export const IGroupApi = Symbol("IGroupApi");

export interface IGroupApi {
  getGroups(
    filter: GroupFilter,
    page: number,
    pageSize: number,
  ): Promise<Group[]>;
  addGroup(name: string): Promise<GroupMeta>;
  editGroup(
    group: GroupMeta,
    editTimestamp?: boolean,
    editGlobalId?: boolean,
  ): Promise<void>;
  deleteGroup(group: GroupMeta): Promise<void>;
  // Only the model ids are consumed, so callers that merely have ids do not
  // need to fabricate full Model objects.
  addModelsToGroup(
    group: GroupMeta,
    models: Pick<Model, "id">[],
  ): Promise<void>;
  removeModelsFromGroup(models: Pick<Model, "id">[]): Promise<void>;
  getGroupCount(include_ungrouped_models: boolean): Promise<number>;
}

// Fetches the full group list by draining the paged stream. A single oversized
// request is not an option: the server caps page_size at MAX_PAGE_SIZE, so
// anything past the first page would be lost.
export async function getAllGroups(api: IGroupApi): Promise<Group[]> {
  const all: Group[] = [];
  for await (const page of groupStream(
    api,
    defaultGroupFilter(),
    MAX_PAGE_SIZE,
  )) {
    all.push(...page);
  }
  return all;
}

export async function* groupStream(
  groupApi: IGroupApi,
  filter: GroupFilter,
  pageSize: number,
): AsyncGenerator<Group[]> {
  yield* pagedStream((pageNumber) =>
    groupApi.getGroups(filter, pageNumber, pageSize),
  );
}

export interface IGroupStreamManager {
  setSearchText(text: string | null): void;
  setOrderBy(order_by: GroupOrderBy): void;
  setFileTypes(fileTypes: FileType[]): void;
  fetch(): Promise<Group[]>;
}

export class PredefinedGroupStreamManager implements IGroupStreamManager {
  private groups: Group[];
  private textSearch: string | null = null;
  private fileTypes: FileType[] | null = null;
  private orderBy: GroupOrderBy = GroupOrderBy.CreatedDesc;
  private alreadyFetched: boolean = false;

  constructor(groups: Group[]) {
    this.groups = groups;
  }

  setSearchText(text: string | null): void {
    this.textSearch = text?.toLowerCase() ?? null;
    this.alreadyFetched = false;
  }

  setOrderBy(order_by: GroupOrderBy): void {
    this.orderBy = order_by;
    this.alreadyFetched = false;
  }

  setFileTypes(fileTypes: FileType[]): void {
    this.fileTypes = normalizeFileTypeFilter(fileTypes);
    this.alreadyFetched = false;
  }

  async fetch(): Promise<Group[]> {
    if (this.alreadyFetched) {
      return [];
    }

    this.alreadyFetched = true;

    const filter = this.groups.filter(
      (group) =>
        (!this.textSearch ||
          group.meta.name.toLowerCase().includes(this.textSearch) ||
          group.models.some((model) =>
            modelMatchesSearch(model, this.textSearch!),
          )) &&
        (!this.fileTypes ||
          group.models.some((model) =>
            this.fileTypes!.includes(model.blob.filetype),
          )),
    );

    return filter.sort(groupOrderByComparator(this.orderBy));
  }
}

export class GroupStreamManager
  extends GeneratorStreamManager<Group, GroupFilter>
  implements IGroupStreamManager
{
  private groupApi: IGroupApi;
  private pageSize: number;

  constructor(
    groupApi: IGroupApi,
    filter: GroupFilter = defaultGroupFilter(),
    pageSize: number = 50,
  ) {
    super(filter);
    this.groupApi = groupApi;
    this.pageSize = pageSize;
    this.regenerate();
  }

  protected makeGenerator(): AsyncGenerator<Group[]> {
    return groupStream(this.groupApi, this.filter, this.pageSize);
  }
}

export async function getGroupById(
  groupApi: IGroupApi,
  groupId: number,
): Promise<Group | null> {
  const groups = await groupApi.getGroups(
    defaultGroupFilter({ groupIds: [groupId] }),
    1,
    1,
  );
  if (groups.length === 0) {
    return null;
  }
  return groups[0];
}
