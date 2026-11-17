import { expect, it, vi } from "vitest";
import {
  defaultGroupFilter,
  type DeletedGroup,
  type IGroupApi,
} from "../shared/group_api";
import { configuration } from "#lib/configuration.svelte";
import { DemoGroupApi } from "./group";
import { mockGroups, mockModels, modelGroupMap } from "./mock_data";

function recoverable(group: DeletedGroup | undefined) {
  if (!group || group.deletionOperationId === null) {
    throw new Error("Expected a recoverable deleted group");
  }
  return { ...group, deletionOperationId: group.deletionOperationId };
}

it("restores only former members that are still ungrouped", async () => {
  const api: IGroupApi = new DemoGroupApi();
  const [first, moved] = [...mockModels.values()].slice(0, 2);
  const original = [first, moved].map((model) => ({
    model,
    group: model.group,
    groupId: modelGroupMap.get(model.id),
  }));
  const source = await api.addGroup("Recovery test source");
  const destination = await api.addGroup("Recovery test destination");

  try {
    await api.addModelsToGroup(source, [first, moved]);
    await api.deleteGroup(source);
    expect(mockModels.has(first.id)).toBe(true);
    expect(mockModels.has(moved.id)).toBe(true);
    expect(modelGroupMap.has(first.id)).toBe(false);
    expect(modelGroupMap.has(moved.id)).toBe(false);
    expect(api.getTrashedGroups).toBeTypeOf("function");
    expect(api.restoreGroups).toBeTypeOf("function");
    expect(
      (await api.getTrashedGroups!()).filter((group) => group.id === source.id),
    ).toHaveLength(1);
    const deleted = recoverable(
      (await api.getTrashedGroups!()).find((group) => group.id === source.id),
    );

    await api.addModelsToGroup(destination, [moved]);
    await api.restoreGroups!([deleted]);
    expect(modelGroupMap.get(first.id)).toBe(source.id);
    expect(modelGroupMap.get(moved.id)).toBe(destination.id);
    expect(
      (await api.getTrashedGroups!()).some((group) => group.id === source.id),
    ).toBe(false);
    await api.restoreGroups!([deleted]);
    expect(modelGroupMap.get(first.id)).toBe(source.id);
    expect(modelGroupMap.get(moved.id)).toBe(destination.id);
  } finally {
    for (const { model, group, groupId } of original) {
      model.group = group;
      if (groupId === undefined) modelGroupMap.delete(model.id);
      else modelGroupMap.set(model.id, groupId);
    }
    mockGroups.delete(source.id);
    mockGroups.delete(destination.id);
  }
});

it("rejects restoring an older deletion after the same group is deleted again", async () => {
  vi.useFakeTimers({ toFake: ["Date"] });
  const api = new DemoGroupApi();
  const group = await api.addGroup("Repeated deletion");
  try {
    await api.deleteGroup(group);
    const first = recoverable(
      (await api.getTrashedGroups()).find((item) => item.id === group.id),
    );
    await api.restoreGroups([first]);
    await api.deleteGroup(group);
    const second = recoverable(
      (await api.getTrashedGroups()).find((item) => item.id === group.id),
    );
    expect(first.deleted.getTime()).toBe(second.deleted.getTime());
    expect(first.groupGlobalId).toBe(group.uniqueGlobalId);
    expect(second.groupGlobalId).toBe(group.uniqueGlobalId);
    expect(first.deletionOperationId).toBeTruthy();
    expect(second.deletionOperationId).toBeTruthy();
    expect(first.deletionOperationId).not.toBe(second.deletionOperationId);
    await expect(
      api.restoreGroups([{ ...second, groupGlobalId: "wrong-group" }]),
    ).rejects.toThrow();
    await expect(api.restoreGroups([first])).rejects.toThrow();
    expect(
      (await api.getTrashedGroups()).find((item) => item.id === group.id),
    ).toEqual(second);
    expect(mockGroups.has(group.id)).toBe(false);
    await api.restoreGroups([second]);
    expect(mockGroups.has(group.id)).toBe(true);
  } finally {
    const current = (await api.getTrashedGroups()).find(
      (item) => item.id === group.id,
    );
    if (current) await api.restoreGroups([recoverable(current)]);
    mockGroups.delete(group.id);
    vi.useRealTimers();
  }
});

it("hides expired groups and rejects restoring their old Trash entry", async () => {
  vi.useFakeTimers({ toFake: ["Date"] });
  const before = configuration.trash_retention_days;
  configuration.trash_retention_days = 1;
  const api = new DemoGroupApi();
  const group = await api.addGroup("Expiring group");
  try {
    await api.deleteGroup(group);
    const deleted = recoverable(
      (await api.getTrashedGroups()).find((item) => item.id === group.id),
    );
    vi.setSystemTime(Date.now() + 2 * 86400000);
    expect(
      (await api.getTrashedGroups()).some((item) => item.id === group.id),
    ).toBe(false);
    await expect(api.restoreGroups([deleted])).rejects.toThrow();
    expect(mockGroups.has(group.id)).toBe(false);
    vi.setSystemTime(deleted.deleted);
    await api.restoreGroups([deleted]);
  } finally {
    configuration.trash_retention_days = before;
    mockGroups.delete(group.id);
    vi.useRealTimers();
  }
});

it("rejects a deleted destination without hiding or moving any models", async () => {
  const api = new DemoGroupApi();
  const [grouped, ungrouped] = [...mockModels.values()].slice(0, 2);
  const original = [grouped, ungrouped].map((model) => ({
    model,
    group: model.group,
    groupId: modelGroupMap.get(model.id),
  }));
  const source = await api.addGroup("Current location");
  const destination = await api.addGroup("Deleted destination");
  try {
    await api.addModelsToGroup(source, [grouped]);
    await api.removeModelsFromGroup([ungrouped]);
    await api.deleteGroup(destination);

    await expect(
      api.addModelsToGroup(destination, [grouped, { id: -999999 }, ungrouped]),
    ).rejects.toThrow();

    expect(mockModels.has(grouped.id)).toBe(true);
    expect(mockModels.has(ungrouped.id)).toBe(true);
    expect(modelGroupMap.get(grouped.id)).toBe(source.id);
    expect(grouped.group?.id).toBe(source.id);
    expect(modelGroupMap.has(ungrouped.id)).toBe(false);
    expect(ungrouped.group).toBeNull();
    const groups = await api.getGroups(
      defaultGroupFilter({
        modelIds: [grouped.id, ungrouped.id],
        includeUngroupedModels: true,
      }),
      1,
      100,
    );
    expect(
      groups.flatMap((group) => group.models.map((model) => model.id)).sort(),
    ).toEqual([grouped.id, ungrouped.id].sort());
  } finally {
    const deletedDestination = (await api.getTrashedGroups()).find(
      (group) => group.id === destination.id,
    );
    if (deletedDestination)
      await api.restoreGroups([recoverable(deletedDestination)]);
    for (const { model, group, groupId } of original) {
      model.group = group;
      if (groupId === undefined) modelGroupMap.delete(model.id);
      else modelGroupMap.set(model.id, groupId);
    }
    mockGroups.delete(source.id);
    mockGroups.delete(destination.id);
  }
});
