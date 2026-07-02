import type { Blob } from "./blob_api";
import type { GroupMeta } from "./group_api";
import type { LabelMeta } from "./label_api";
import { GeneratorStreamManager } from "./stream_manager";

export interface ModelFlags {
  printed: boolean;
  favorite: boolean;
}

export function stringArrayToModelFlags(flagList: string[]): ModelFlags {
  const flags = {
    printed: false,
    favorite: false,
  };

  flagList.forEach((flag) => {
    switch (flag) {
      case "Printed":
        flags.printed = true;
        break;
      case "Favorite":
        flags.favorite = true;
        break;
    }
  });

  return flags;
}

export interface Model {
  id: number;
  name: string;
  blob: Blob;
  link: string | null;
  description: string | null;
  added: Date;
  lastModified: Date;
  group: GroupMeta | null;
  labels: LabelMeta[];
  flags: ModelFlags;
  uniqueGlobalId: string;
}

export function createModelInstance(
  id: number,
  name: string,
  blob: Blob,
  link: string | null,
  description: string | null,
  added: string,
  last_modified: string,
  group: GroupMeta | null,
  labels: LabelMeta[],
  flags: string[],
  uniqueGlobalId: string,
): Model {
  return {
    id,
    name,
    blob,
    link,
    description,
    added: new Date(added),
    lastModified: new Date(last_modified),
    group,
    labels,
    flags: stringArrayToModelFlags(flags),
    uniqueGlobalId: uniqueGlobalId,
  };
}

export enum ModelOrderBy {
  AddedAsc = "AddedAsc",
  AddedDesc = "AddedDesc",
  NameAsc = "NameAsc",
  NameDesc = "NameDesc",
  SizeAsc = "SizeAsc",
  SizeDesc = "SizeDesc",
  ModifiedAsc = "ModifiedAsc",
  ModifiedDesc = "ModifiedDesc",
}

export const IModelApi = Symbol("IModelApi");

export interface IModelApi {
  getModels(
    model_ids: number[] | null,
    group_ids: number[] | null,
    label_ids: number[] | null,
    order_by: ModelOrderBy,
    text_search: string | null,
    page: number,
    page_size: number,
    flags: ModelFlags | null,
  ): Promise<Model[]>;
  editModel(
    model: Model,
    editTimestamp?: boolean,
    editGlobalId?: boolean,
  ): Promise<void>;
  deleteModel(model: Model): Promise<void>;
  deleteModels(models: Model[]): Promise<void>;
  getModelCount(flags: ModelFlags | null): Promise<number>;
}

// Largest page size the backends accept (mirrors db::MAX_PAGE_SIZE; the web
// server rejects anything larger with a 400).
export const MAX_PAGE_SIZE = 1000;

// Fetches the full model list (optionally filtered by labels) by draining the
// paged stream. A single oversized request is not an option: the server caps
// page_size at MAX_PAGE_SIZE, so anything past the first page would be lost.
export async function getAllModels(
  api: IModelApi,
  labelIds: number[] | null = null,
): Promise<Model[]> {
  const all: Model[] = [];
  for await (const page of modelStream(
    api,
    null,
    null,
    labelIds,
    ModelOrderBy.ModifiedDesc,
    null,
    null,
    MAX_PAGE_SIZE,
  )) {
    all.push(...page);
  }
  return all;
}

export async function* modelStream(
  modelApi: IModelApi,
  modelIds: number[] | null,
  groupIds: number[] | null,
  labelIds: number[] | null,
  orderBy: ModelOrderBy,
  textSearch: string | null,
  flags: ModelFlags | null,
  pageSize: number = 50,
): AsyncGenerator<Model[]> {
  let page = 1;
  let prefetchNextTask: Promise<Model[]> | null = null;

  const fetchPage = (pageNumber: number) =>
    modelApi.getModels(
      modelIds,
      groupIds,
      labelIds,
      orderBy,
      textSearch,
      pageNumber,
      pageSize,
      flags,
    );

  while (true) {
    prefetchNextTask ??= fetchPage(page);

    const models = await prefetchNextTask;
    if (models.length === 0) {
      break;
    }

    page += 1;
    prefetchNextTask = fetchPage(page);

    yield models;
  }
}

export interface IModelStreamManager {
  setSearchText(text: string | null): void;
  setOrderBy(order_by: ModelOrderBy): void;
  fetch(): Promise<Model[]>;
  getAll(): Promise<Model[]>;
}

export class PredefinedModelStreamManager implements IModelStreamManager {
  private models: Model[];
  private textSearch: string | null = null;
  private orderBy: ModelOrderBy = ModelOrderBy.AddedDesc;
  private pageSize: number;
  private fetchIndex: number = 0;
  // Filtered + sorted view, computed lazily and reused across page fetches.
  // Invalidated whenever the search text or sort order changes.
  private sortedFiltered: Model[] | null = null;

  constructor(models: Model[], pageSize: number = 50) {
    this.models = models;
    this.pageSize = pageSize;
  }

  setSearchText(text: string | null): void {
    this.textSearch = text?.toLowerCase() ?? null;
    this.fetchIndex = 0;
    this.sortedFiltered = null;
  }

  setOrderBy(order_by: ModelOrderBy): void {
    this.orderBy = order_by;
    this.fetchIndex = 0;
    this.sortedFiltered = null;
  }

  private computeSortedFiltered(): Model[] {
    const filtered = !this.textSearch
      ? this.models
      : this.models.filter(
          (model) =>
            model.name.toLowerCase().includes(this.textSearch!) ||
            (model.description?.toLowerCase().includes(this.textSearch!) ??
              false),
        );

    // Copy before sorting so we never mutate the caller-owned `this.models`.
    return [...filtered].sort((a, b) => {
      switch (this.orderBy) {
        case ModelOrderBy.AddedAsc:
          return a.added.getTime() - b.added.getTime();
        case ModelOrderBy.AddedDesc:
          return b.added.getTime() - a.added.getTime();
        case ModelOrderBy.NameAsc:
          return a.name.localeCompare(b.name);
        case ModelOrderBy.NameDesc:
          return b.name.localeCompare(a.name);
        case ModelOrderBy.SizeAsc:
          return a.blob.size - b.blob.size;
        case ModelOrderBy.SizeDesc:
          return b.blob.size - a.blob.size;
        default:
          return 0;
      }
    });
  }

  async fetch(): Promise<Model[]> {
    if (this.sortedFiltered === null) {
      this.sortedFiltered = this.computeSortedFiltered();
    }

    if (this.fetchIndex >= this.sortedFiltered.length) {
      return [];
    }

    const paged = this.sortedFiltered.slice(
      this.fetchIndex,
      this.fetchIndex + this.pageSize,
    );
    this.fetchIndex += this.pageSize;

    return paged;
  }

  async getAll(): Promise<Model[]> {
    return this.models;
  }
}

export class ModelStreamManager
  extends GeneratorStreamManager<Model, ModelOrderBy>
  implements IModelStreamManager
{
  private modelApi: IModelApi;
  private modelIds: number[] | null;
  private groupIds: number[] | null;
  private labelIds: number[] | null;
  private flags: ModelFlags | null;
  private pageSize: number;

  constructor(
    modelApi: IModelApi,
    modelIds: number[] | null,
    groupIds: number[] | null,
    labelIds: number[] | null,
    flags: ModelFlags | null,
    pageSize: number = 50,
  ) {
    super(ModelOrderBy.AddedDesc);
    this.modelApi = modelApi;
    this.modelIds = modelIds;
    this.groupIds = groupIds;
    this.labelIds = labelIds;
    this.flags = flags;
    this.pageSize = pageSize;
    this.regenerate();
  }

  protected makeGenerator(): AsyncGenerator<Model[]> {
    return modelStream(
      this.modelApi,
      this.modelIds,
      this.groupIds,
      this.labelIds,
      this.orderBy,
      this.textSearch,
      this.flags,
      this.pageSize,
    );
  }

  async getAll(): Promise<Model[]> {
    return [];
  }
}
