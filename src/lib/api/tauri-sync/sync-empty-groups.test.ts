import { afterEach, expect, it, vi } from "vitest";
import { currentUser } from "#lib/configuration.svelte";
import { resetSyncState } from "#lib/sync.svelte";
import { getContainer, resetContainer } from "../dependency_injection";
import {
  createGroupInstance,
  createGroupMetaInstance,
  IGroupApi,
  type Group,
  type GroupMeta,
} from "../shared/group_api";
import { syncGroups } from "./sync-groups";

function emptyGroup(
  id: number,
  modified: string,
  gid = "empty-group-gid",
): Group {
  return createGroupInstance(
    createGroupMetaInstance(id, "Empty folder", modified, modified, gid),
    [],
    [],
    null,
    [],
  );
}

function groupApi(groups: Group[], nextId: number) {
  return {
    getGroups: vi.fn(async (_filter: unknown, page: number) =>
      page === 1 ? groups : [],
    ),
    addGroup: vi.fn(async (name: string) =>
      createGroupMetaInstance(
        nextId,
        name,
        "2025-06-01T00:00:00Z",
        "2025-06-01T00:00:00Z",
        "new-gid",
      ),
    ),
    addModelsToGroup: vi.fn(
      async (_group: GroupMeta, _models: unknown[]) => {},
    ),
    removeModelsFromGroup: vi.fn(async (_models: unknown[]) => {}),
    editGroup: vi.fn(
      async (
        _group: GroupMeta,
        _timestamp?: boolean,
        _globalId?: boolean,
      ) => {},
    ),
    deleteGroup: vi.fn(async (_group: GroupMeta) => {}),
  };
}

afterEach(() => {
  resetContainer();
  currentUser.lastSync = null;
  resetSyncState();
});

it("uploads a newly created empty group with its identity and no model attachments", async () => {
  currentUser.lastSync = new Date("2025-01-01T00:00:00Z");
  const source = emptyGroup(4, "2025-06-01T00:00:00Z");
  const local = groupApi([source], 4);
  const remote = groupApi([], 44);
  getContainer().addSingleton(IGroupApi, local);

  await syncGroups(remote as unknown as IGroupApi, new Map(), new Map());

  expect(remote.addGroup).toHaveBeenCalledExactlyOnceWith("Empty folder");
  expect(remote.editGroup).toHaveBeenCalledExactlyOnceWith(
    expect.objectContaining({ id: 44, uniqueGlobalId: "empty-group-gid" }),
    true,
    true,
  );
  expect(remote.addModelsToGroup).not.toHaveBeenCalled();
  expect(remote.deleteGroup).not.toHaveBeenCalled();
  expect(local.deleteGroup).not.toHaveBeenCalled();
});

it("does not delete a pre-existing local empty group merely because the remote has never listed it", async () => {
  currentUser.lastSync = new Date("2025-01-01T00:00:00Z");
  const local = groupApi([emptyGroup(4, "2024-01-01T00:00:00Z")], 4);
  const remote = groupApi([], 44);
  getContainer().addSingleton(IGroupApi, local);

  await syncGroups(remote as unknown as IGroupApi, new Map(), new Map());

  expect(local.deleteGroup).not.toHaveBeenCalled();
  expect(remote.addGroup).toHaveBeenCalledExactlyOnceWith("Empty folder");
});

it("does not delete a pre-existing remote empty group merely because the local side has never listed it", async () => {
  currentUser.lastSync = new Date("2025-01-01T00:00:00Z");
  const local = groupApi([], 4);
  const remote = groupApi([emptyGroup(44, "2024-01-01T00:00:00Z")], 44);
  getContainer().addSingleton(IGroupApi, local);

  await syncGroups(remote as unknown as IGroupApi, new Map(), new Map());

  expect(remote.deleteGroup).not.toHaveBeenCalled();
  expect(local.addGroup).toHaveBeenCalledExactlyOnceWith("Empty folder");
});

it("syncs metadata for matched empty groups without changing model attachments", async () => {
  currentUser.lastSync = new Date("2025-01-01T00:00:00Z");
  const local = groupApi([emptyGroup(4, "2025-06-01T00:00:00Z")], 4);
  const remote = groupApi([emptyGroup(44, "2025-03-01T00:00:00Z")], 44);
  getContainer().addSingleton(IGroupApi, local);

  await syncGroups(remote as unknown as IGroupApi, new Map(), new Map());

  expect(remote.editGroup).toHaveBeenCalledExactlyOnceWith(
    expect.objectContaining({ id: 44, uniqueGlobalId: "empty-group-gid" }),
    true,
    false,
  );
  expect(remote.removeModelsFromGroup).not.toHaveBeenCalled();
  expect(remote.addModelsToGroup).not.toHaveBeenCalled();
  expect(local.deleteGroup).not.toHaveBeenCalled();
  expect(remote.deleteGroup).not.toHaveBeenCalled();
});
