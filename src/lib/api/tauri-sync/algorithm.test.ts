/**
 * Regression tests for sync algorithm (computeDifferences, forceApplyFieldToObject).
 * Locks behaviour before refactors or sync logic changes.
 */

import { describe, it, expect } from "vitest";
import {
  computeDifferences,
  defaultSyncResult,
  forceApplyFieldToObject,
  type DiffableItem,
} from "./algorithm";

function item(id: string, lastModified: Date): DiffableItem {
  return { uniqueGlobalId: id, lastModified };
}

describe("computeDifferences", () => {
  const lastSynced = new Date("2025-01-15T12:00:00Z");

  it("returns empty result when both sides are empty", () => {
    const result = computeDifferences([], [], lastSynced);
    expect(result).toEqual(defaultSyncResult<DiffableItem>());
  });

  it("puts local-only item modified after lastSynced into toUpload", () => {
    const local = [item("a", new Date("2025-01-20T00:00:00Z"))];
    const result = computeDifferences(local, [], lastSynced);
    expect(result.toUpload).toHaveLength(1);
    expect(result.toUpload[0].uniqueGlobalId).toBe("a");
    expect(result.toDeleteLocal).toHaveLength(0);
    expect(result.toDownload).toHaveLength(0);
  });

  it("puts local-only item modified before lastSynced into toDeleteLocal", () => {
    const local = [item("a", new Date("2025-01-10T00:00:00Z"))];
    const result = computeDifferences(local, [], lastSynced);
    expect(result.toDeleteLocal).toHaveLength(1);
    expect(result.toDeleteLocal[0].uniqueGlobalId).toBe("a");
    expect(result.toUpload).toHaveLength(0);
  });

  it("puts local-only item with lastModified equal to lastSynced into toUpload", () => {
    const local = [item("a", new Date(lastSynced.getTime()))];
    const result = computeDifferences(local, [], lastSynced);
    expect(result.toUpload).toHaveLength(1);
    expect(result.toUpload[0].uniqueGlobalId).toBe("a");
    expect(result.toDeleteLocal).toHaveLength(0);
  });

  it("puts server-only item modified after lastSynced into toDownload", () => {
    const server = [item("b", new Date("2025-01-20T00:00:00Z"))];
    const result = computeDifferences([], server, lastSynced);
    expect(result.toDownload).toHaveLength(1);
    expect(result.toDownload[0].uniqueGlobalId).toBe("b");
    expect(result.toDeleteServer).toHaveLength(0);
  });

  it("puts server-only item modified before lastSynced into toDeleteServer", () => {
    const server = [item("b", new Date("2025-01-10T00:00:00Z"))];
    const result = computeDifferences([], server, lastSynced);
    expect(result.toDeleteServer).toHaveLength(1);
    expect(result.toDeleteServer[0].uniqueGlobalId).toBe("b");
    expect(result.toDownload).toHaveLength(0);
  });

  it("puts server-only item with lastModified equal to lastSynced into toDownload", () => {
    const server = [item("b", new Date(lastSynced.getTime()))];
    const result = computeDifferences([], server, lastSynced);
    expect(result.toDownload).toHaveLength(1);
    expect(result.toDownload[0].uniqueGlobalId).toBe("b");
    expect(result.toDeleteServer).toHaveLength(0);
  });

  it("puts pair in syncToServer when local is newer than server", () => {
    const t = new Date("2025-01-20T00:00:00Z");
    const older = new Date("2025-01-18T00:00:00Z");
    const local = [item("c", t)];
    const server = [item("c", older)];
    const result = computeDifferences(local, server, lastSynced);
    expect(result.syncToServer).toHaveLength(1);
    expect(result.syncToServer[0].local.uniqueGlobalId).toBe("c");
    expect(result.syncToServer[0].server.uniqueGlobalId).toBe("c");
    expect(result.syncToLocal).toHaveLength(0);
  });

  it("puts pair in syncToLocal when server is newer than local", () => {
    const t = new Date("2025-01-20T00:00:00Z");
    const older = new Date("2025-01-18T00:00:00Z");
    const local = [item("d", older)];
    const server = [item("d", t)];
    const result = computeDifferences(local, server, lastSynced);
    expect(result.syncToLocal).toHaveLength(1);
    expect(result.syncToLocal[0].local.uniqueGlobalId).toBe("d");
    expect(result.syncToLocal[0].server.uniqueGlobalId).toBe("d");
    expect(result.syncToServer).toHaveLength(0);
  });

  it("leaves pair out of sync lists when lastModified times are equal", () => {
    const t = new Date("2025-01-20T00:00:00Z");
    const local = [item("e", t)];
    const server = [item("e", new Date(t.getTime()))];
    const result = computeDifferences(local, server, lastSynced);
    expect(result.syncToServer).toHaveLength(0);
    expect(result.syncToLocal).toHaveLength(0);
    expect(result.toUpload).toHaveLength(0);
    expect(result.toDownload).toHaveLength(0);
  });
});

describe("forceApplyFieldToObject", () => {
  it("merges object with result of extractor", () => {
    type Foo = { id: string; name: string };
    const objects: Foo[] = [
      { id: "1", name: "Alice" },
      { id: "2", name: "Bob" },
    ];
    const extractor = (obj: Foo): DiffableItem => ({
      uniqueGlobalId: obj.id,
      lastModified: new Date(1000),
    });
    const result = forceApplyFieldToObject(objects, extractor);
    expect(result).toHaveLength(2);
    expect(result[0]).toEqual({
      id: "1",
      name: "Alice",
      uniqueGlobalId: "1",
      lastModified: new Date(1000),
    });
    expect(result[1]).toEqual({
      id: "2",
      name: "Bob",
      uniqueGlobalId: "2",
      lastModified: new Date(1000),
    });
  });
});
