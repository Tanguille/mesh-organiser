/**
 * Regression tests for DemoGroupApi.getGroups, locking the grouped and
 * ungrouped model collection behaviour before/after extracting the shared
 * collectGroupModels helper. The two collection loops differ only in their
 * membership predicate (grouped: modelGroupMap.get(id) === groupId; ungrouped:
 * !modelGroupMap.has(id)), so these tests assert that membership, filtering and
 * label collection are unchanged for the representative mockModels fixture.
 *
 * Model flags in mock_data are randomised, so the assertions deliberately rely
 * only on the deterministic aspects (membership, names, labels) the refactor
 * touches.
 */

import { describe, it, expect } from "vitest";
import { GroupOrderBy } from "../shared/group_api";
import { DemoGroupApi } from "./group";
import {
  mockGroups,
  mockModels,
  modelGroupMap,
  modelLabelsMap,
} from "./mock_data";

const api = new DemoGroupApi();

// All models that are assigned to a group in the fixture.
function groupedModelIds(): Set<number> {
  const ids = new Set<number>();
  modelGroupMap.forEach((_groupId, modelId) => ids.add(modelId));

  return ids;
}

describe("DemoGroupApi.getGroups grouped collection", () => {
  it("collects only the models whose modelGroupMap entry matches the group", async () => {
    // Arrange
    const groupedIds = groupedModelIds();
    // Act
    const groups = await api.getGroups(
      null,
      null,
      null,
      GroupOrderBy.NameAsc,
      null,
      1,
      100,
      false,
    );
    // Assert: every returned group is a real group, and each contained model
    // is mapped to exactly that group id in modelGroupMap.
    expect(groups.length).toBeGreaterThan(0);
    for (const group of groups) {
      expect(mockGroups.has(group.meta.id)).toBe(true);
      for (const model of group.models) {
        expect(groupedIds.has(model.id)).toBe(true);
        expect(modelGroupMap.get(model.id)).toBe(group.meta.id);
      }
    }
  });

  it("accumulates the union of member label ids onto the group", async () => {
    // Arrange: expected labels per group derived directly from the fixture.
    const expectedLabelsByGroup = new Map<number, Set<number>>();
    modelGroupMap.forEach((groupId, modelId) => {
      const labels = expectedLabelsByGroup.get(groupId) ?? new Set<number>();
      (modelLabelsMap.get(modelId) ?? []).forEach((lid) => labels.add(lid));
      expectedLabelsByGroup.set(groupId, labels);
    });
    // Act
    const groups = await api.getGroups(
      null,
      null,
      null,
      GroupOrderBy.NameAsc,
      null,
      1,
      100,
      false,
    );
    // Assert
    for (const group of groups) {
      const expected = expectedLabelsByGroup.get(group.meta.id) ?? new Set();
      const actual = new Set(group.labels.map((label) => label.id));
      expect(actual).toEqual(expected);
    }
  });

  it("applies the text_search predicate to member models", async () => {
    // Act: search for a primitive that lives inside the grouped branch.
    const groups = await api.getGroups(
      null,
      null,
      null,
      GroupOrderBy.NameAsc,
      "sphere",
      1,
      100,
      false,
    );
    // Assert: only matching models survive within the returned groups.
    for (const group of groups) {
      for (const model of group.models) {
        expect(model.name.toLowerCase()).toContain("sphere");
      }
    }
  });
});

describe("DemoGroupApi.getGroups ungrouped collection", () => {
  it("turns each model without a group into its own group, partitioning all models", async () => {
    // Arrange
    const groupedIds = groupedModelIds();
    const ungroupedIds = new Set<number>();
    mockModels.forEach((_model, modelId) => {
      if (!modelGroupMap.has(modelId)) ungroupedIds.add(modelId);
    });
    // Act
    const groups = await api.getGroups(
      null,
      null,
      null,
      GroupOrderBy.NameAsc,
      null,
      1,
      100,
      true,
    );
    // Assert: ungrouped models appear as singleton groups with a negated id,
    // and grouped + ungrouped membership partitions every mock model.
    const ungroupedGroups = groups.filter((group) => group.meta.id < 0);
    const seenUngrouped = new Set<number>();
    for (const group of ungroupedGroups) {
      expect(group.models).toHaveLength(1);
      const model = group.models[0];
      expect(ungroupedIds.has(model.id)).toBe(true);
      expect(group.meta.id).toBe(model.id * -1);
      seenUngrouped.add(model.id);
    }
    expect(seenUngrouped).toEqual(ungroupedIds);

    // The two predicates are mutually exclusive and cover all models.
    for (const modelId of groupedIds) {
      expect(ungroupedIds.has(modelId)).toBe(false);
    }
    expect(groupedIds.size + ungroupedIds.size).toBe(mockModels.size);
  });

  it("excludes ungrouped singleton groups when include_ungrouped_models is false", async () => {
    // Act
    const groups = await api.getGroups(
      null,
      null,
      null,
      GroupOrderBy.NameAsc,
      null,
      1,
      100,
      false,
    );
    // Assert
    expect(groups.every((group) => group.meta.id >= 0)).toBe(true);
  });

  it("filters ungrouped models by model_ids", async () => {
    // Arrange: pick a single ungrouped model id from the fixture.
    const targetId = [...mockModels.keys()].find(
      (modelId) => !modelGroupMap.has(modelId),
    );
    expect(targetId).toBeDefined();
    // Act
    const groups = await api.getGroups(
      [targetId!],
      null,
      null,
      GroupOrderBy.NameAsc,
      null,
      1,
      100,
      true,
    );
    // Assert: the only ungrouped singleton present is the requested model.
    const ungroupedGroups = groups.filter((group) => group.meta.id < 0);
    expect(ungroupedGroups).toHaveLength(1);
    expect(ungroupedGroups[0].models[0].id).toBe(targetId);
  });
});
