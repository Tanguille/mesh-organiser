/**
 * Characterization tests for WebModelApi's trash requests. Method, url and body
 * are the contract with the web server's /models/trash and /models/restore
 * routes; a mismatch fails silently against the Rust backend.
 */

import { describe, expect, it, vi } from "vitest";
import type { Model } from "../shared/model_api";
import type { RawModel } from "../shared/raw_model";
import {
  HttpMethod,
  type IServerRequestApi,
} from "../shared/server_request_api";
import { WebModelApi } from "./model";

function rawModel(id: number, deleted: string | null): RawModel {
  return {
    id,
    name: `model ${id}`,
    blob: {
      id,
      sha256: "sha",
      filetype: "stl",
      size: 1,
      added: "2024-01-01T00:00:00Z",
    },
    link: null,
    description: null,
    added: "2024-01-01T00:00:00Z",
    last_modified: "2024-01-01T00:00:00Z",
    group: null,
    labels: [],
    flags: [],
    unique_global_id: `gid-${id}`,
    deleted,
  };
}

function createApi(response?: unknown) {
  const request = vi.fn().mockResolvedValue(response);
  const requestApi = { request } as unknown as IServerRequestApi;

  return { request, api: new WebModelApi(requestApi) };
}

describe("WebModelApi trash", () => {
  it("getTrashedModels GETs /models/trash and parses each model's deleted timestamp", async () => {
    const { request, api } = createApi([
      rawModel(4, "2026-10-21T10:00:00Z"),
      rawModel(2, "2026-10-01T00:00:00Z"),
    ]);

    const models = await api.getTrashedModels();

    expect(request.mock.calls).toEqual([["/models/trash", HttpMethod.GET]]);
    expect(models.map((model) => [model.id, model.deleted])).toEqual([
      [4, new Date("2026-10-21T10:00:00Z")],
      [2, new Date("2026-10-01T00:00:00Z")],
    ]);
  });

  it("restoreModels POSTs the model ids in the given order to /models/restore", async () => {
    const { request, api } = createApi();

    await api.restoreModels([{ id: 9 }, { id: 3 }] as Model[]);

    expect(request.mock.calls).toEqual([
      ["/models/restore", HttpMethod.POST, { model_ids: [9, 3] }],
    ]);
  });
});
