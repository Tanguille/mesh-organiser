import { describe, expect, it } from "vitest";
import { createGroupInstance, createGroupMetaInstance } from "./group_api";
import {
  chunkIdsForCommaSeparatedQuery,
  mergeGroupsFromChunkedResponses,
  MODEL_IDS_STR_SAFE_CHUNK_CHARS,
} from "./group_query_chunks";
import type { Model } from "./model_api";
import { FileType } from "./blob_api";

function minimalModel(id: number): Model {
  return {
    id,
    name: `m${id}`,
    blob: {
      id,
      sha256: "",
      filetype: FileType.STL,
      size: 0,
      added: new Date(),
    },
    link: null,
    description: null,
    added: new Date(),
    lastModified: new Date(),
    group: null,
    labels: [],
    flags: { favorite: false, printed: false },
    uniqueGlobalId: "",
  };
}

describe("chunkIdsForCommaSeparatedQuery", () => {
  it("returns empty list for empty input", () => {
    expect(chunkIdsForCommaSeparatedQuery([], 100)).toEqual([]);
  });

  it("splits when comma-joined length exceeds budget", () => {
    const ids = [1, 12, 123];
    const joined = ids.join(",");
    expect(joined.length).toBeGreaterThan(4);
    const chunks = chunkIdsForCommaSeparatedQuery(ids, 4);
    expect(chunks.length).toBeGreaterThan(1);
    expect(chunks.flat().sort((a, b) => a - b)).toEqual(ids);
  });

  it("keeps a single chunk under MODEL_IDS_STR_SAFE_CHUNK_CHARS for small sets", () => {
    const ids = [1, 2, 3];
    const chunks = chunkIdsForCommaSeparatedQuery(
      ids,
      MODEL_IDS_STR_SAFE_CHUNK_CHARS,
    );
    expect(chunks).toEqual([ids]);
  });
});

describe("mergeGroupsFromChunkedResponses", () => {
  it("unions models for the same group id", () => {
    const meta = createGroupMetaInstance(
      1,
      "G",
      new Date().toISOString(),
      new Date().toISOString(),
      "gid",
    );
    const a = createGroupInstance(
      meta,
      [minimalModel(10)],
      [],
      null,
      [],
    );
    const b = createGroupInstance(
      meta,
      [minimalModel(20)],
      [],
      null,
      [],
    );
    const merged = mergeGroupsFromChunkedResponses([a, b]);
    expect(merged).toHaveLength(1);
    expect(merged[0].models.map((m) => m.id).sort()).toEqual([10, 20]);
  });
});
