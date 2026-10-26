/**
 * Characterization tests for DemoModelApi.deleteModels: each deleted model must
 * be dropped from the model, group-membership and label-membership maps, and
 * every other model must stay.
 */

import { afterEach, describe, expect, it } from "vitest";
import { DemoModelApi } from "./model";
import { mockModels, modelGroupMap, modelLabelsMap } from "./mock_data";

const initialModels = new Map(mockModels);
const initialGroups = new Map(modelGroupMap);
const initialLabels = new Map(modelLabelsMap);

afterEach(() => {
  mockModels.clear();
  modelGroupMap.clear();
  modelLabelsMap.clear();
  for (const [id, model] of initialModels) mockModels.set(id, model);
  for (const [id, group] of initialGroups) modelGroupMap.set(id, group);
  for (const [id, labels] of initialLabels) modelLabelsMap.set(id, labels);
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
