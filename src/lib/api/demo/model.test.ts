/**
 * Characterization test for DemoModelApi.deleteModels, pinned before
 * deleteModel is removed from IModelApi: deleteModels currently delegates to
 * deleteModel per model, so the removal must keep dropping each model from the
 * model, group-membership and label-membership maps.
 */

import { describe, expect, it } from "vitest";
import { DemoModelApi } from "./model";
import { mockModels, modelGroupMap, modelLabelsMap } from "./mock_data";

describe("DemoModelApi.deleteModels", () => {
  it("removes each model from the models, group and label maps and leaves the rest", async () => {
    const groupedId = [...modelGroupMap.keys()][0];
    const labelledId = [...modelLabelsMap.keys()][0];
    const deleted = [mockModels.get(groupedId)!, mockModels.get(labelledId)!];
    const remainingIds = [...mockModels.keys()].filter(
      (id) => id !== groupedId && id !== labelledId,
    );
    const groupCountBefore = modelGroupMap.size;
    const labelCountBefore = modelLabelsMap.size;
    const modelCountBefore = mockModels.size;

    await new DemoModelApi().deleteModels(deleted);

    for (const id of [groupedId, labelledId]) {
      expect(mockModels.has(id)).toBe(false);
      expect(modelGroupMap.has(id)).toBe(false);
      expect(modelLabelsMap.has(id)).toBe(false);
    }
    expect(mockModels.size).toBe(modelCountBefore - 2);
    expect(modelGroupMap.size).toBe(groupCountBefore - 1);
    expect(modelLabelsMap.size).toBe(labelCountBefore - 1);
    expect([...mockModels.keys()]).toEqual(remainingIds);
  });
});
