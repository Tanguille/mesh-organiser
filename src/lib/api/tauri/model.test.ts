/**
 * Characterization tests for the Tauri ModelApi trash commands. Command names
 * and argument keys are the contract with src-tauri's invoke handlers; Tauri
 * maps the camelCase `modelIds` onto the Rust `model_ids` parameter.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import type { Model } from "../shared/model_api";
import type { RawModel } from "../shared/raw_model";
import { ModelApi } from "./model";

const invoke = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const trashedRawModel: RawModel = {
  id: 5,
  name: "benchy",
  blob: {
    id: 1,
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
  unique_global_id: "gid-5",
  deleted: "2026-10-22T18:45:00Z",
};

describe("ModelApi trash", () => {
  afterEach(() => {
    invoke.mockReset();
  });

  it("getTrashedModels invokes get_trashed_models without arguments and parses deleted", async () => {
    invoke.mockResolvedValue([trashedRawModel]);

    const models = await new ModelApi().getTrashedModels();

    expect(invoke.mock.calls).toEqual([["get_trashed_models"]]);
    expect(models).toHaveLength(1);
    expect(models[0].id).toBe(5);
    expect(models[0].deleted).toEqual(new Date("2026-10-22T18:45:00Z"));
  });

  it("restoreModels invokes restore_models with camelCase modelIds", async () => {
    invoke.mockResolvedValue(undefined);

    await new ModelApi().restoreModels([{ id: 7 }, { id: 2 }] as Model[]);

    expect(invoke.mock.calls).toEqual([
      ["restore_models", { modelIds: [7, 2] }],
    ]);
  });
});
