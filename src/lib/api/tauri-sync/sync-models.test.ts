/**
 * Pins what syncModels does with each uploaded model: it detaches the model
 * from its server group only when the upload's import put it in a group other
 * than the local one. A model the server restored from its trash keeps its
 * real group, so detaching it would make group sync drop it on both sides.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { currentUser } from "#lib/configuration.svelte";
import { getContainer, resetContainer } from "../dependency_injection";
import { createBlobInstance, type IBlobApi } from "../shared/blob_api";
import { createGroupMetaInstance, IGroupApi } from "../shared/group_api";
import {
  createModelInstance,
  IModelApi,
  type Model,
  type ModelFilter,
} from "../shared/model_api";
import { ITauriImportApi } from "../shared/tauri_import_api";
import { syncModels } from "./sync-models";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  uploadModels: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("../tauri-online/tauri_import", () => ({
  uploadModels: mocks.uploadModels,
}));
vi.mock("../tauri/tauri_import", () => ({ downloadFile: vi.fn() }));
vi.mock("$app/navigation", () => ({ goto: vi.fn() }));
vi.mock("$app/paths", () => ({ resolve: (path: string) => path }));

const LOCAL_BLOB_ID = 3;
const SERVER_MODEL_ID = 42;
const BLOB_PATH = "/blobs/benchy.stl";

function group(uniqueGlobalId: string) {
  return createGroupMetaInstance(
    7,
    uniqueGlobalId,
    "2025-06-01T00:00:00Z",
    "2025-06-01T00:00:00Z",
    uniqueGlobalId,
  );
}

function model(id: number, groupGlobalId: string | null): Model {
  return createModelInstance(
    id,
    "benchy",
    createBlobInstance(LOCAL_BLOB_ID, "sha", "stl", 1, "2025-06-01T00:00:00Z"),
    null,
    null,
    "2025-06-01T00:00:00Z",
    // After lastSynced and absent on the server, so the sync uploads it.
    "2025-06-01T00:00:00Z",
    groupGlobalId ? group(groupGlobalId) : null,
    [],
    [],
    "model-gid",
  );
}

/** Runs a sync that uploads one local model, which the server reports back in `serverGroupGlobalId`. */
async function syncOneUpload(
  localGroupGlobalId: string | null,
  serverGroupGlobalId: string | null,
) {
  const localModel = model(1, localGroupGlobalId);
  const serverModelApi = {
    // Empty for the initial listing; the post-upload lookup filters on the new id.
    getModels: vi.fn(async (filter: ModelFilter) =>
      filter.modelIds ? [model(SERVER_MODEL_ID, serverGroupGlobalId)] : [],
    ),
    editModel: vi.fn().mockResolvedValue(undefined),
  };
  const serverGroupApi = {
    removeModelsFromGroup: vi.fn().mockResolvedValue(undefined),
  };
  const container = getContainer();
  container.addSingleton(IModelApi, {
    getModels: vi.fn(async (_filter: ModelFilter, page: number) =>
      page === 1 ? [localModel] : [],
    ),
  });
  container.addSingleton(IGroupApi, {});
  container.addSingleton(ITauriImportApi, {});
  mocks.invoke.mockResolvedValue([
    { blob_id: LOCAL_BLOB_ID, blob_path: BLOB_PATH },
  ]);
  mocks.uploadModels.mockResolvedValue({
    import_state: {},
    uploaded_models: [
      {
        path: BLOB_PATH,
        group_set: null,
        group_name: null,
        model_ids: [SERVER_MODEL_ID],
      },
    ],
  });

  await syncModels(
    serverModelApi as unknown as IModelApi,
    serverGroupApi as unknown as IGroupApi,
    {} as IBlobApi,
  );

  return { localModel, serverModelApi, serverGroupApi };
}

describe("syncModels upload", () => {
  afterEach(() => {
    resetContainer();
    mocks.invoke.mockReset();
    mocks.uploadModels.mockReset();
    currentUser.lastSync = null;
  });

  it.each([
    ["the server model has no group", "local", null, false],
    ["the server restored the model into its own group", "kept", "kept", false],
    ["the import grouped a locally ungrouped model", null, "auto", true],
    ["the import grouped it apart from its local group", "local", "auto", true],
  ])(
    "detaches only when the server group differs: %s",
    async (_description, localGroup, serverGroup, detaches) => {
      currentUser.lastSync = new Date("2025-01-01T00:00:00Z");

      const { localModel, serverModelApi, serverGroupApi } =
        await syncOneUpload(localGroup, serverGroup);

      expect(localModel.id).toBe(SERVER_MODEL_ID);
      expect(serverModelApi.getModels).toHaveBeenCalledWith(
        expect.objectContaining({ modelIds: [SERVER_MODEL_ID] }),
        1,
        1,
      );
      expect(serverGroupApi.removeModelsFromGroup.mock.calls).toEqual(
        detaches ? [[[localModel]]] : [],
      );
      expect(serverModelApi.editModel.mock.calls).toEqual([
        [localModel, true, true],
      ]);
    },
  );
});
