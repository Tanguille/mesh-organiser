import { describe, expect, it, vi } from "vitest";
import { WebGroupApi } from "./group";
import {
  HttpMethod,
  type IServerRequestApi,
} from "../shared/server_request_api";
import { GroupOrderBy } from "../shared/group_api";
import type { RawGroup } from "../tauri/group";
import {
  chunkIdsForCommaSeparatedQuery,
  MODEL_IDS_STR_SAFE_CHUNK_CHARS,
} from "../shared/group_query_chunks";

const iso = "2020-01-01T00:00:00.000Z";

function readQueryInt(
  data: Record<string, unknown> | undefined,
  key: string,
): number {
  const value = data?.[key];
  if (typeof value !== "number" || !Number.isInteger(value)) {
    throw new Error(
      `expected query field ${key} to be an integer, got ${typeof value}`,
    );
  }

  return value;
}

function readModelIdsStr(data: Record<string, unknown> | undefined): string {
  const raw = data?.model_ids_str;
  if (raw === undefined || raw === null) {
    return "";
  }
  if (typeof raw !== "string") {
    throw new Error(`expected model_ids_str to be a string, got ${typeof raw}`);
  }

  return raw;
}

function rawGroup(groupId: number, modelId: number): RawGroup {
  return {
    meta: {
      id: groupId,
      name: `G${groupId}`,
      created: iso,
      last_modified: iso,
      resource_id: null,
      unique_global_id: `ug${groupId}`,
    },
    models: [
      {
        id: modelId,
        name: `M${modelId}`,
        blob: {
          id: modelId,
          sha256: "a".repeat(64),
          filetype: "STL",
          size: 0,
          added: iso,
        },
        link: null,
        description: null,
        added: iso,
        last_modified: iso,
        group: null,
        labels: [],
        flags: [],
        unique_global_id: `um${modelId}`,
      },
    ],
    labels: [],
    resource: null,
    flags: [],
  } satisfies RawGroup;
}

/** Enough numeric ids that `join(',')` exceeds `MODEL_IDS_STR_SAFE_CHUNK_CHARS` (4096). */
function wideModelIdList(): number[] {
  return Array.from({ length: 500 }, (_, i) => 100_000_000 + i);
}

describe("WebGroupApi.getGroups chunked path", () => {
  it("paginates per id-chunk until a short page then merges across chunks", async () => {
    const modelIds = wideModelIdList();
    const chunks = chunkIdsForCommaSeparatedQuery(
      modelIds,
      MODEL_IDS_STR_SAFE_CHUNK_CHARS,
    );
    expect(chunks.length).toBeGreaterThan(1);

    const calls: { page: number; model_ids_str: string }[] = [];
    const firstKey = chunks[0].join(",");

    const request = vi.fn(
      async (
        endpoint: string,
        _method: HttpMethod,
        data?: Record<string, unknown>,
      ): Promise<RawGroup[]> => {
        if (endpoint !== "/groups") {
          throw new Error(`unexpected endpoint ${endpoint}`);
        }
        const page = readQueryInt(data, "page");
        const modelIdsStr = readModelIdsStr(data);
        calls.push({ page, model_ids_str: modelIdsStr });

        if (modelIdsStr === firstKey) {
          if (page === 1) {
            return [rawGroup(1, 1), rawGroup(2, 2)];
          }
          if (page === 2) {
            return [rawGroup(3, 3)];
          }

          throw new Error(`unexpected page ${page} for first chunk`);
        }

        if (chunks.slice(1).some((chunk) => modelIdsStr === chunk.join(","))) {
          if (page !== 1) {
            throw new Error(`unexpected page ${page} for tail chunk`);
          }

          return [];
        }

        throw new Error(`unknown model_ids_str (len=${modelIdsStr.length})`);
      },
    );

    const api = new WebGroupApi({
      baseUrl: "",
      request,
      requestBinary: vi.fn(),
      sendBinary: vi.fn(),
    } as unknown as IServerRequestApi);

    const groups = await api.getGroups(
      modelIds,
      null,
      null,
      GroupOrderBy.NameAsc,
      null,
      1,
      2,
      false,
    );

    const firstChunkCalls = calls.filter((c) => c.model_ids_str === firstKey);
    expect(firstChunkCalls).toHaveLength(2);
    expect(firstChunkCalls[0].page).toBe(1);
    expect(firstChunkCalls[1].page).toBe(2);

    const mergedIds = new Set(groups.map((g) => g.meta.id));
    expect(mergedIds).toEqual(new Set([1, 2, 3]));
  });
});
