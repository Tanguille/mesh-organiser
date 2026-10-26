/**
 * Pins what syncModels does with each uploaded model: it detaches the model
 * from its server group only when the upload's import put it in a group other
 * than the local one. A model the server restored from its trash keeps its
 * real group, so detaching it would make group sync drop it on both sides.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { currentUser } from "#lib/configuration.svelte";
import {
  globalSyncState,
  resetSyncState,
  SyncStage,
  SyncStep,
} from "#lib/sync.svelte";
import { getContainer, resetContainer } from "../dependency_injection";
import { createBlobInstance, type IBlobApi } from "../shared/blob_api";
import { createGroupMetaInstance, IGroupApi } from "../shared/group_api";
import {
  createModelInstance,
  IModelApi,
  MAX_PAGE_SIZE,
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
    getModels: vi.fn(async (filter: ModelFilter, page: number) =>
      filter.modelIds && page === 1
        ? [model(SERVER_MODEL_ID, serverGroupGlobalId)]
        : [],
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

async function syncMultipleUploads(
  localModels: Model[],
  serverModels: Model[],
) {
  const paths = localModels.map((localModel, index) => {
    localModel.blob.id = LOCAL_BLOB_ID + index;
    localModel.blob.sha256 = `sha-${index}`;
    localModel.uniqueGlobalId = `model-gid-${index}`;

    return {
      blob_id: localModel.blob.id,
      blob_path: `/blobs/model-${index}.stl`,
    };
  });
  const serverModelApi = {
    getModels: vi.fn(
      async (filter: ModelFilter, page: number, pageSize: number) => {
        const ids = filter.modelIds;
        const matching = ids
          ? serverModels.filter((serverModel) => ids.includes(serverModel.id))
          : [];

        return matching.slice((page - 1) * pageSize, page * pageSize);
      },
    ),
    editModel: vi.fn().mockResolvedValue(undefined),
  };
  const serverGroupApi = {
    removeModelsFromGroup: vi.fn().mockResolvedValue(undefined),
  };
  const container = getContainer();
  container.addSingleton(IModelApi, {
    getModels: vi.fn(
      async (_filter: ModelFilter, page: number, pageSize: number) =>
        localModels.slice((page - 1) * pageSize, page * pageSize),
    ),
  });
  container.addSingleton(IGroupApi, {});
  container.addSingleton(ITauriImportApi, {});
  mocks.invoke.mockResolvedValue(paths);
  mocks.uploadModels.mockResolvedValue({
    import_state: {},
    uploaded_models: paths.map((path, index) => ({
      path: path.blob_path,
      group_set: null,
      group_name: null,
      model_ids: [serverModels[index].id],
    })),
  });

  await syncModels(
    serverModelApi as unknown as IModelApi,
    serverGroupApi as unknown as IGroupApi,
    {} as IBlobApi,
  );

  return { serverModelApi, serverGroupApi };
}

describe("syncModels upload", () => {
  afterEach(() => {
    resetContainer();
    mocks.invoke.mockReset();
    mocks.uploadModels.mockReset();
    currentUser.lastSync = null;
    resetSyncState();
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
      expect(
        serverModelApi.getModels.mock.calls.filter(
          ([filter]) => filter.modelIds !== null,
        ),
      ).toEqual([
        [
          expect.objectContaining({ modelIds: [SERVER_MODEL_ID] }),
          1,
          MAX_PAGE_SIZE,
        ],
      ]);
      expect(serverGroupApi.removeModelsFromGroup.mock.calls).toEqual(
        detaches ? [[[localModel]]] : [],
      );
      expect(serverModelApi.editModel.mock.calls).toEqual([
        [localModel, true, true],
      ]);
    },
  );

  it("looks up all uploaded models in one request and preserves group decisions", async () => {
    currentUser.lastSync = new Date("2025-01-01T00:00:00Z");
    const localModels = [model(1, "kept"), model(2, "local"), model(3, null)];
    const serverModels = [
      model(42, "kept"),
      model(43, "auto"),
      model(44, null),
    ];
    const { serverModelApi, serverGroupApi } = await syncMultipleUploads(
      localModels,
      serverModels,
    );

    const ids = serverModels.map((serverModel) => serverModel.id);
    const lookupCalls = serverModelApi.getModels.mock.calls.filter(
      ([filter]) => filter.modelIds !== null,
    );
    expect(lookupCalls).toEqual([
      [expect.objectContaining({ modelIds: ids }), 1, MAX_PAGE_SIZE],
    ]);
    expect(localModels.map((localModel) => localModel.id)).toEqual(ids);
    expect(serverGroupApi.removeModelsFromGroup.mock.calls).toEqual([
      [[localModels[1]]],
    ]);
    expect(serverModelApi.editModel).toHaveBeenCalledTimes(3);
    for (const localModel of localModels) {
      expect(serverModelApi.editModel).toHaveBeenCalledWith(
        localModel,
        true,
        true,
      );
    }
    expect(globalSyncState).toEqual({
      stage: SyncStage.Models,
      step: SyncStep.Upload,
      processableItems: 3,
      processedItems: 3,
    });
  });

  it("uses one first-page lookup per bounded upload batch and completes every edit", async () => {
    currentUser.lastSync = new Date("2025-01-01T00:00:00Z");
    const localModels = Array.from({ length: MAX_PAGE_SIZE + 1 }, (_, index) =>
      model(index + 1, null),
    );
    const serverModels = localModels.map((localModel) =>
      model(localModel.id + MAX_PAGE_SIZE, null),
    );

    const { serverModelApi, serverGroupApi } = await syncMultipleUploads(
      localModels,
      serverModels,
    );

    const lookupCalls = serverModelApi.getModels.mock.calls.filter(
      ([filter]) => filter.modelIds !== null,
    );
    expect(lookupCalls.map(([filter]) => filter.modelIds?.length)).toEqual([
      MAX_PAGE_SIZE,
      1,
    ]);
    expect(lookupCalls.flatMap(([filter]) => filter.modelIds ?? [])).toEqual(
      serverModels.map((serverModel) => serverModel.id),
    );
    expect(lookupCalls.map(([, page]) => page)).toEqual([1, 1]);
    for (const [filter, , pageSize] of lookupCalls) {
      expect(filter.modelIds!.length).toBeLessThanOrEqual(MAX_PAGE_SIZE);
      expect(pageSize).toBe(MAX_PAGE_SIZE);
    }
    expect(serverModelApi.editModel).toHaveBeenCalledTimes(localModels.length);
    expect(
      serverModelApi.editModel.mock.calls
        .map(([uploaded]) => uploaded.id)
        .sort((left, right) => left - right),
    ).toEqual(serverModels.map((serverModel) => serverModel.id));
    expect(serverGroupApi.removeModelsFromGroup).not.toHaveBeenCalled();
    expect(globalSyncState).toEqual({
      stage: SyncStage.Models,
      step: SyncStep.Upload,
      processableItems: localModels.length,
      processedItems: localModels.length,
    });
  });
});
