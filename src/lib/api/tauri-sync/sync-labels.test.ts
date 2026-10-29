/**
 * Pins that uploading a new label awaits setKeywordsOnLabel: a rejection must
 * fail the sync before the label's models, children and global id are pushed,
 * instead of escaping as an unhandled rejection.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { currentUser } from "#lib/configuration.svelte";
import { resetSyncState } from "#lib/sync.svelte";
import { getContainer, resetContainer } from "../dependency_injection";
import {
  createLabelMetaInstance,
  ILabelApi,
  type Label,
} from "../shared/label_api";
import { IModelApi } from "../shared/model_api";
import { syncLabels } from "./sync-labels";

function label(): Label {
  return {
    // After lastSynced and absent on the server, so the sync uploads it.
    meta: createLabelMetaInstance(
      1,
      "PLA",
      0xff0000,
      "2025-06-01T00:00:00Z",
      "label-gid",
    ),
    children: [],
    effectiveLabels: [],
    hasParent: false,
    modelCount: 0,
    groupCount: 0,
    selfModelCount: 0,
    selfGroupCount: 0,
  };
}

describe("syncLabels upload", () => {
  afterEach(() => {
    resetContainer();
    currentUser.lastSync = null;
    resetSyncState();
  });

  it("fails the sync when setting the uploaded label's keywords rejects", async () => {
    currentUser.lastSync = new Date("2025-01-01T00:00:00Z");
    const keywordsError = new Error("keywords rejected");
    const serverLabelApi = {
      getLabels: vi.fn().mockResolvedValue([]),
      addLabel: vi
        .fn()
        .mockResolvedValue(
          createLabelMetaInstance(
            9,
            "PLA",
            0xff0000,
            "2025-06-01T00:00:00Z",
            "server-gid",
          ),
        ),
      setKeywordsOnLabel: vi.fn(() => {
        const rejected = Promise.reject(keywordsError);
        // Marked handled up front so the unawaited call fails this test on the
        // assertion below, not as a stray unhandled rejection.
        rejected.catch(() => {});

        return rejected;
      }),
      addLabelToModels: vi.fn().mockResolvedValue(undefined),
      setChildrenOnLabel: vi.fn().mockResolvedValue(undefined),
      editLabel: vi.fn().mockResolvedValue(undefined),
    };
    const container = getContainer();
    container.addSingleton(ILabelApi, {
      getLabels: vi.fn().mockResolvedValue([label()]),
      getKeywordsForLabel: vi.fn().mockResolvedValue(["filament"]),
    });
    container.addSingleton(IModelApi, {
      getModels: vi.fn().mockResolvedValue([]),
    });

    const sync = syncLabels(
      {} as IModelApi,
      serverLabelApi as unknown as ILabelApi,
      new Map(),
      new Map(),
    );

    await expect(sync).rejects.toBe(keywordsError);
    expect(serverLabelApi.setKeywordsOnLabel).toHaveBeenCalledWith(
      expect.objectContaining({ id: 9 }),
      ["filament"],
    );
    expect(serverLabelApi.addLabelToModels).not.toHaveBeenCalled();
    expect(serverLabelApi.editLabel).not.toHaveBeenCalled();
  });
});
