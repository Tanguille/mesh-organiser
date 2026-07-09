/**
 * Regression tests for applySyncResult, the shared runner used by the three
 * identical sync files (groups, labels, resources). Locks the exact branch
 * order and the `.length > 0` gating before refactors or sync logic changes.
 */

import { describe, it, expect } from "vitest";
import {
  applySyncResult,
  defaultSyncResult,
  type DiffableItem,
  type ResourceSet,
  type SyncResult,
  type SyncResultHandlers,
} from "./algorithm";

function item(id: string): DiffableItem {
  return { uniqueGlobalId: id, lastModified: new Date(0) };
}

function pair(id: string): ResourceSet<DiffableItem> {
  return { local: item(id), server: item(id) };
}

// Records the order in which handlers fire so we can assert the exact
// toUpload -> toDownload -> syncToServer -> syncToLocal -> deleteServer ->
// deleteLocal sequence the inline branches used.
function trackingHandlers(calls: string[]): SyncResultHandlers<DiffableItem> {
  return {
    upload: async () => {
      calls.push("upload");
    },
    download: async () => {
      calls.push("download");
    },
    syncToServer: async () => {
      calls.push("syncToServer");
    },
    syncToLocal: async () => {
      calls.push("syncToLocal");
    },
    deleteServer: async () => {
      calls.push("deleteServer");
    },
    deleteLocal: async () => {
      calls.push("deleteLocal");
    },
  };
}

describe("applySyncResult", () => {
  it("invokes all six handlers in the fixed order when every bucket is populated", async () => {
    const syncState: SyncResult<DiffableItem> = {
      toUpload: [item("u")],
      toDownload: [item("d")],
      syncToServer: [pair("ss")],
      syncToLocal: [pair("sl")],
      toDeleteServer: [item("ds")],
      toDeleteLocal: [item("dl")],
    };
    const calls: string[] = [];

    await applySyncResult(syncState, trackingHandlers(calls));

    expect(calls).toEqual([
      "upload",
      "download",
      "syncToServer",
      "syncToLocal",
      "deleteServer",
      "deleteLocal",
    ]);
  });

  it("skips every handler when all buckets are empty", async () => {
    const syncState = defaultSyncResult<DiffableItem>();
    const calls: string[] = [];

    await applySyncResult(syncState, trackingHandlers(calls));

    expect(calls).toEqual([]);
  });

  it("only invokes handlers whose bucket is non-empty, preserving order", async () => {
    const syncState: SyncResult<DiffableItem> = {
      ...defaultSyncResult<DiffableItem>(),
      toDownload: [item("d")],
      toDeleteLocal: [item("dl")],
    };
    const calls: string[] = [];

    await applySyncResult(syncState, trackingHandlers(calls));

    expect(calls).toEqual(["download", "deleteLocal"]);
  });

  it("passes the matching bucket to each handler", async () => {
    const upload = [item("u")];
    const download = [item("d")];
    const syncToServer = [pair("ss")];
    const syncToLocal = [pair("sl")];
    const deleteServer = [item("ds")];
    const deleteLocal = [item("dl")];
    const syncState: SyncResult<DiffableItem> = {
      toUpload: upload,
      toDownload: download,
      syncToServer,
      syncToLocal,
      toDeleteServer: deleteServer,
      toDeleteLocal: deleteLocal,
    };
    const received: Record<string, unknown> = {};

    await applySyncResult(syncState, {
      upload: async (arg) => {
        received.upload = arg;
      },
      download: async (arg) => {
        received.download = arg;
      },
      syncToServer: async (arg) => {
        received.syncToServer = arg;
      },
      syncToLocal: async (arg) => {
        received.syncToLocal = arg;
      },
      deleteServer: async (arg) => {
        received.deleteServer = arg;
      },
      deleteLocal: async (arg) => {
        received.deleteLocal = arg;
      },
    });

    expect(received.upload).toBe(upload);
    expect(received.download).toBe(download);
    expect(received.syncToServer).toBe(syncToServer);
    expect(received.syncToLocal).toBe(syncToLocal);
    expect(received.deleteServer).toBe(deleteServer);
    expect(received.deleteLocal).toBe(deleteLocal);
  });
});
