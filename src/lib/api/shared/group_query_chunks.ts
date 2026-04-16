import {
  sortGroupsByOrderBy,
  type Group,
  type GroupOrderBy,
} from "./group_api";
import type { Model } from "./model_api";

/**
 * Conservative cap for the comma-separated `model_ids_str` segment so the full
 * GET URL stays under common ~8KiB limits alongside other query fields.
 * Chunking uses `String.length` (UTF-16 code units); for decimal ids and commas
 * that is the same numeric budget as UTF-8 `str::len()` on the wire.
 * Must equal `MODEL_IDS_STR_SAFE_CHUNK_CHARS` in `web/src/query_bounds.rs`.
 */
export const MODEL_IDS_STR_SAFE_CHUNK_CHARS = 4096;

/**
 * Split numeric ids into chunks where each chunk's comma-joined string length
 * is at most `maxChars` (ASCII digits and commas → UTF-8 byte length).
 */
export function chunkIdsForCommaSeparatedQuery(
  ids: number[],
  maxChars: number,
): number[][] {
  if (ids.length === 0) {
    return [];
  }

  const chunks: number[][] = [];
  let current: number[] = [];
  let currentLen = 0;

  for (const id of ids) {
    const segment = current.length === 0 ? String(id) : `,${id}`;

    if (currentLen + segment.length > maxChars && current.length > 0) {
      chunks.push(current);
      current = [id];
      currentLen = String(id).length;
    } else {
      current.push(id);
      currentLen += segment.length;
    }
  }

  if (current.length > 0) {
    chunks.push(current);
  }

  return chunks;
}

function mergeTwoGroupsByModels(firstGroup: Group, secondGroup: Group): Group {
  const byId = new Map<number, Model>(
    firstGroup.models.map((model) => [model.id, model]),
  );
  for (const model of secondGroup.models) {
    byId.set(model.id, model);
  }

  return {
    ...firstGroup,
    models: [...byId.values()],
  };
}

/**
 * When `model_ids_str` is loaded in multiple requests, the same group can
 * appear more than once with disjoint model subsets — merge into one row per
 * group id (union models).
 */
export function mergeGroupsFromChunkedResponses(groups: Group[]): Group[] {
  const byMetaId = new Map<number, Group>();
  const order: number[] = [];

  for (const groupRow of groups) {
    const groupId = groupRow.meta.id;
    const existing = byMetaId.get(groupId);
    if (!existing) {
      byMetaId.set(groupId, groupRow);
      order.push(groupId);
    } else {
      byMetaId.set(groupId, mergeTwoGroupsByModels(existing, groupRow));
    }
  }

  return order.map((groupId) => byMetaId.get(groupId)!);
}

export function finalizeChunkedGroupList(
  groups: Group[],
  orderBy: GroupOrderBy,
): Group[] {
  const merged = mergeGroupsFromChunkedResponses(groups);
  sortGroupsByOrderBy(merged, orderBy);
  return merged;
}
