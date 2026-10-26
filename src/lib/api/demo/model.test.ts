/**
 * Characterization tests for DemoModelApi.deleteModels: each deleted model must
 * be dropped from the model, group-membership and label-membership maps, and
 * every other model must stay.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { defaultModelFilter } from "../shared/model_api";
import { DemoGroupApi } from "./group";
import { DemoLabelApi } from "./label";
import { DemoModelApi } from "./model";
import {
  mockGroups,
  mockLabels,
  mockModels,
  modelGroupMap,
  modelLabelsMap,
} from "./mock_data";

const models = structuredClone(mockModels);
const groups = structuredClone(mockGroups);
const labels = structuredClone(mockLabels);
const groupMemberships = structuredClone(modelGroupMap);
const labelMemberships = structuredClone(modelLabelsMap);

function resetMap<T>(target: Map<number, T>, initial: Map<number, T>) {
  target.clear();
  for (const [id, value] of structuredClone(initial)) target.set(id, value);
}

afterEach(async () => {
  const api = new DemoModelApi();
  const trashed = await api.getTrashedModels();
  if (trashed.length > 0) await api.restoreModels(trashed);
  vi.useRealTimers();
  resetMap(mockModels, models);
  resetMap(mockGroups, groups);
  resetMap(mockLabels, labels);
  resetMap(modelGroupMap, groupMemberships);
  resetMap(modelLabelsMap, labelMemberships);
});

describe("DemoModelApi.deleteModels", () => {
  it.each(["distinct ids", "same id"])(
    "removes %s from all maps and leaves the rest",
    async (selection) => {
      const groupedId = [...modelGroupMap.keys()][0];
      const labelledId =
        selection === "same id"
          ? groupedId
          : [...mockModels.keys()].find((id) => id !== groupedId)!;
      const group = modelGroupMap.get(groupedId)!;
      const labels = [...modelLabelsMap.values()][0];
      for (const id of [groupedId, labelledId]) {
        modelGroupMap.set(id, group);
        modelLabelsMap.set(id, labels);
      }
      const deleted = [mockModels.get(groupedId)!, mockModels.get(labelledId)!];
      const deletedIds = new Set(deleted.map((model) => model.id));
      const modelsBefore = [...mockModels];
      const groupsBefore = [...modelGroupMap];
      const labelsBefore = [...modelLabelsMap];

      await new DemoModelApi().deleteModels(deleted);

      expect([...mockModels]).toEqual(
        modelsBefore.filter(([id]) => !deletedIds.has(id)),
      );
      expect([...modelGroupMap]).toEqual(
        groupsBefore.filter(([id]) => !deletedIds.has(id)),
      );
      expect([...modelLabelsMap]).toEqual(
        labelsBefore.filter(([id]) => !deletedIds.has(id)),
      );
    },
  );
});

describe("DemoModelApi trash", () => {
  it("preserves model data and memberships when another API instance restores it", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-10-25T12:00:00Z"));
    const api = new DemoModelApi();
    const model = mockModels.get(1)!;
    const group = mockGroups.get(1)!;
    const label = mockLabels.get(1)!;
    await new DemoGroupApi().addModelsToGroup(group, [model]);
    await new DemoLabelApi().setLabelsOnModel([label], model);
    model.description = "Keep this description";
    model.flags = { printed: true, favorite: true };
    const expected = structuredClone(model);
    const count = await api.getModelCount(null);

    await api.deleteModels([model]);

    expect(mockModels.has(model.id)).toBe(false);
    expect(modelGroupMap.has(model.id)).toBe(false);
    expect(modelLabelsMap.has(model.id)).toBe(false);
    expect(await api.getModelCount(null)).toBe(count - 1);
    const otherApi = new DemoModelApi();
    const trashed = await otherApi.getTrashedModels();
    expect(trashed).toEqual([
      { ...expected, deleted: new Date("2026-10-25T12:00:00Z") },
    ]);

    await otherApi.restoreModels(trashed);

    expect(await api.getTrashedModels()).toEqual([]);
    expect(await api.getModelCount(null)).toBe(count);
    expect(
      await api.getModels(defaultModelFilter({ modelIds: [model.id] }), 1, 10),
    ).toEqual([expected]);
    expect(modelGroupMap.get(model.id)).toBe(group.id);
    expect(modelLabelsMap.get(model.id)).toEqual([label.id]);
  });

  it("lists the most recently deleted models first", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    const api = new DemoModelApi();
    const first = mockModels.get(1)!;
    const second = mockModels.get(2)!;
    vi.setSystemTime(new Date("2026-10-25T12:00:00Z"));
    await api.deleteModels([first]);
    vi.setSystemTime(new Date("2026-10-25T13:00:00Z"));
    await api.deleteModels([second]);

    expect((await api.getTrashedModels()).map((model) => model.id)).toEqual([
      2, 1,
    ]);
  });

  it("does not recreate groups or labels deleted while a model was trashed", async () => {
    const api = new DemoModelApi();
    const groupApi = new DemoGroupApi();
    const labelApi = new DemoLabelApi();
    const model = mockModels.get(1)!;
    const group = mockGroups.get(1)!;
    const label = mockLabels.get(1)!;
    const keptLabel = await labelApi.addLabel("Keep", "#112233");
    await groupApi.addModelsToGroup(group, [model]);
    await labelApi.setLabelsOnModel([label, keptLabel], model);
    await api.deleteModels([model]);
    await groupApi.deleteGroup(group);
    await labelApi.deleteLabel(label);

    await api.restoreModels([model]);

    expect(mockModels.get(model.id)?.group).toBeNull();
    expect(mockModels.get(model.id)?.labels).toEqual([keptLabel]);
    expect(modelGroupMap.has(model.id)).toBe(false);
    expect(modelLabelsMap.get(model.id)).toEqual([keptLabel.id]);
    expect(mockGroups.has(group.id)).toBe(false);
    expect(mockLabels.has(label.id)).toBe(false);
  });

  it("does not attach a restored model to replacement groups or labels reusing deleted ids", async () => {
    const api = new DemoModelApi();
    const groupApi = new DemoGroupApi();
    const labelApi = new DemoLabelApi();
    const model = mockModels.get(1)!;
    const group = mockGroups.get(1)!;
    const label = mockLabels.get(1)!;
    await groupApi.addModelsToGroup(group, [model]);
    await labelApi.setLabelsOnModel([label], model);
    await api.deleteModels([model]);
    await groupApi.deleteGroup(group);
    await labelApi.deleteLabel(label);
    const replacementGroup = await groupApi.addGroup("Replacement group");
    const replacementLabel = await labelApi.addLabel(
      "Replacement label",
      "#112233",
    );
    expect(replacementGroup.id).toBe(group.id);
    expect(replacementLabel.id).toBe(label.id);

    await api.restoreModels([model]);

    expect(mockModels.get(model.id)?.group).toBeNull();
    expect(mockModels.get(model.id)?.labels).toEqual([]);
    expect(modelGroupMap.has(model.id)).toBe(false);
    expect(modelLabelsMap.has(model.id)).toBe(false);
  });

  it("restores a repeated model id only once", async () => {
    const api = new DemoModelApi();
    const model = mockModels.get(1)!;
    const expected = structuredClone(model);
    await api.deleteModels([model]);

    await expect(api.restoreModels([model, model])).resolves.toBeUndefined();

    expect(mockModels.get(model.id)).toEqual(expected);
    expect(await api.getTrashedModels()).toEqual([]);
  });

  it.each(["unknown", "live"])(
    "rejects a restore batch containing %s id without restoring its valid models",
    async (invalidKind) => {
      const api = new DemoModelApi();
      const model = mockModels.get(1)!;
      const live = mockModels.get(2)!;
      const invalid = invalidKind === "live" ? live : { ...model, id: 9999 };
      await api.deleteModels([model]);

      await expect(api.restoreModels([model, invalid])).rejects.toThrow();

      expect(mockModels.has(model.id)).toBe(false);
      expect(mockModels.get(live.id)).toEqual(live);
      expect((await api.getTrashedModels()).map((item) => item.id)).toEqual([
        model.id,
      ]);
    },
  );
});
