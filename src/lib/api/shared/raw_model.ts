import { createBlobInstance, type Blob } from "./blob_api";
import {
  createGroupInstance,
  createGroupMetaInstance,
  type Group,
  type GroupMeta,
} from "./group_api";
import {
  createLabelInstance,
  createLabelMetaInstance,
  type Label,
  type LabelMeta,
} from "./label_api";
import {
  createModelInstance,
  type Model,
  type ModelFilter,
  type ModelFlags,
} from "./model_api";
import {
  createResourceMetaInstance,
  type ResourceFlags,
  type ResourceMeta,
} from "./resource_api";
import { createUserInstance, type User } from "./user_api";

/** Raw blob shape (matches tauri blob response). */
export interface RawBlob {
  id: number;
  sha256: string;
  filetype: string;
  size: number;
  added: string;
}

/** Raw group meta shape (matches tauri group response). */
export interface RawGroupMeta {
  id: number;
  name: string;
  created: string;
  last_modified: string;
  resource_id: number | null;
  unique_global_id: string;
}

/** Raw label meta shape (matches tauri label response). */
export interface RawLabelMeta {
  id: number;
  name: string;
  color: number;
  unique_global_id: string;
  last_modified: string;
}

export interface RawModel {
  id: number;
  name: string;
  blob: RawBlob;
  link: string | null;
  description: string | null;
  added: string;
  last_modified: string;
  group: RawGroupMeta | null;
  labels: RawLabelMeta[];
  flags: string[];
  unique_global_id: string;
}

/** Builds query parameters for the web and web-share model endpoints. */
export function buildGetModelsQuery(
  filter: ModelFilter,
  page: number,
  page_size: number,
) {
  return {
    model_ids: filter.modelIds,
    group_ids: filter.groupIds,
    label_ids: filter.labelIds,
    order_by: filter.orderBy,
    text_search: filter.textSearch,
    file_types: filter.fileTypes,
    page,
    page_size,
    model_flags: convertModelFlagsToRaw(filter.flags),
  };
}

export function convertModelFlagsToRaw(
  flags: ModelFlags | null,
): string[] | null {
  if (flags === null) {
    return null;
  }

  const rawFlags: string[] = [];

  if (flags.printed) {
    rawFlags.push("Printed");
  }

  if (flags.favorite) {
    rawFlags.push("Favorite");
  }

  if (rawFlags.length === 0) {
    return null;
  }

  return rawFlags;
}

export function parseRawBlob(raw: RawBlob): Blob {
  return createBlobInstance(
    raw.id,
    raw.sha256,
    raw.filetype,
    raw.size,
    raw.added,
  );
}

export function parseRawGroupMeta(raw: RawGroupMeta): GroupMeta {
  return createGroupMetaInstance(
    raw.id,
    raw.name,
    raw.created,
    raw.last_modified,
    raw.unique_global_id,
  );
}

export interface RawGroup {
  meta: RawGroupMeta;
  models: RawModel[];
  labels: RawLabelMeta[];
  resource: RawResourceMeta | null;
  flags: string[];
}

export function parseRawGroup(raw: RawGroup): Group {
  return createGroupInstance(
    parseRawGroupMeta(raw.meta),
    raw.models.map((model) => parseRawModel(model)),
    raw.labels.map((label) => parseRawLabelMeta(label)),
    raw.resource ? parseRawResourceMeta(raw.resource) : null,
    raw.flags,
  );
}

export function parseRawLabelMeta(raw: RawLabelMeta): LabelMeta {
  return createLabelMetaInstance(
    raw.id,
    raw.name,
    raw.color,
    raw.last_modified,
    raw.unique_global_id,
  );
}

export interface RawLabel {
  meta: RawLabelMeta;
  children: RawLabelMeta[];
  effective_labels: RawLabelMeta[];
  has_parent: boolean;
  model_count: number;
  group_count: number;
  self_model_count: number;
  self_group_count: number;
}

export function parseRawLabel(raw: RawLabel): Label {
  return createLabelInstance(
    parseRawLabelMeta(raw.meta),
    raw.children.map((child) => parseRawLabelMeta(child)),
    raw.effective_labels.map((effective) => parseRawLabelMeta(effective)),
    raw.has_parent,
    raw.model_count,
    raw.group_count,
    raw.self_model_count,
    raw.self_group_count,
  );
}

export interface RawLabelKeyword {
  id: number;
  name: string;
}

export interface RawResourceMeta {
  id: number;
  name: string;
  flags: string[];
  created: string;
  unique_global_id: string;
  last_modified: string;
}

export function parseRawResourceMeta(raw: RawResourceMeta): ResourceMeta {
  return createResourceMetaInstance(
    raw.id,
    raw.name,
    raw.flags,
    raw.created,
    raw.last_modified,
    raw.unique_global_id,
  );
}

export function convertResourceFlagsToRaw(flags: ResourceFlags): string[] {
  const raw_flags: string[] = [];

  if (flags.completed) {
    raw_flags.push("Completed");
  }

  return raw_flags;
}

export function parseRawModel(raw: RawModel): Model {
  return createModelInstance(
    raw.id,
    raw.name,
    parseRawBlob(raw.blob),
    raw.link,
    raw.description,
    raw.added,
    raw.last_modified,
    raw.group ? parseRawGroupMeta(raw.group) : null,
    raw.labels.map((label) => parseRawLabelMeta(label)),
    raw.flags,
    raw.unique_global_id,
  );
}

/** Raw user shape (matches tauri and server user responses). */
export interface RawUser {
  id: number;
  username: string;
  email: string;
  created_at: string;
  permissions: string[];
  sync_url: string | null;
  sync_token: string | null;
  last_sync: string | null;
}

export function parseRawUser(raw: RawUser): User {
  return createUserInstance(
    raw.id,
    raw.username,
    raw.email,
    raw.created_at,
    raw.permissions,
    raw.sync_url,
    raw.sync_token,
    raw.last_sync,
  );
}
