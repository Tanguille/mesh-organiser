import { expect, it } from "vitest";
import {
  defaultGroupFilter,
  getAllGroups,
  getGroupById,
} from "../shared/group_api";
import { DemoGroupApi } from "./group";
import { DemoModelApi } from "./model";
import { mockModels, modelGroupMap } from "./mock_data";

it("lists a newly created empty group as a real destination and resolves it by id", async () => {
  const api = new DemoGroupApi();
  const initialCount = await api.getGroupCount(false);
  const group = await api.addGroup("Empty destination");
  try {
    const groups = await getAllGroups(api);
    expect(groups.find((value) => value.meta.id === group.id)?.models).toEqual(
      [],
    );
    expect(await api.getGroupCount(false)).toBe(groups.length);
    expect(await api.getGroupCount(false)).toBe(initialCount + 1);
    expect((await getGroupById(api, group.id))?.meta.id).toBe(group.id);
  } finally {
    await api.deleteGroup(group);
  }
});

it("does not include an empty group when a model-id filter does not match", async () => {
  const api = new DemoGroupApi();
  const group = await api.addGroup("Empty destination");
  try {
    const groups = await api.getGroups(
      defaultGroupFilter({ modelIds: [-123] }),
      1,
      1000,
    );
    expect(groups.some((value) => value.meta.id === group.id)).toBe(false);
  } finally {
    await api.deleteGroup(group);
  }
});

it("does not list a group whose only model is in the trash", async () => {
  const groupApi = new DemoGroupApi();
  const modelApi = new DemoModelApi();
  const model = [...mockModels.values()].find(
    (item) => !modelGroupMap.has(item.id),
  )!;
  const group = await groupApi.addGroup("Trashed only");
  try {
    await groupApi.addModelsToGroup(group, [model]);
    await modelApi.deleteModels([model]);
    const groups = await getAllGroups(groupApi);
    expect(groups.some((item) => item.meta.id === group.id)).toBe(false);
    expect(await groupApi.getGroupCount(false)).toBe(groups.length);
  } finally {
    if (
      (await modelApi.getTrashedModels()).some((item) => item.id === model.id)
    ) {
      await modelApi.restoreModels([model]);
    }
    await groupApi.deleteGroup(group);
  }
});

it("finds a populated group by its name even when no model name matches", async () => {
  const api = new DemoGroupApi();
  const model = [...mockModels.values()].find(
    (item) => !modelGroupMap.has(item.id),
  )!;
  const group = await api.addGroup("Unique destination xyz");
  try {
    await api.addModelsToGroup(group, [model]);
    const groups = await api.getGroups(
      defaultGroupFilter({ textSearch: "destination xyz" }),
      1,
      1000,
    );
    const found = groups.find((item) => item.meta.id === group.id);
    expect(found).toBeDefined();
    expect(found!.models.map((item) => item.id)).toContain(model.id);
  } finally {
    await api.deleteGroup(group);
  }
});
