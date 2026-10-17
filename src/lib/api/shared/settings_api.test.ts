/**
 * Characterization tests for the settings option lists and the order-option
 * converters. The sort dropdowns and size pickers render these lists in
 * insertion order, and the converters fall back to a default for values
 * persisted by older versions.
 */

import { describe, expect, it } from "vitest";
import { GroupOrderBy } from "./group_api";
import { ModelOrderBy } from "./model_api";
import {
  convertOrderOptionGroupsToEnum,
  convertOrderOptionModelsToEnum,
  GROUP_ORDER_LABELS,
  MODEL_ORDER_LABELS,
  SizeOptionModelsAsList,
  type OrderOptionGroups,
  type OrderOptionModels,
} from "./settings_api";

describe("option lists", () => {
  it("lists the size options in grid-then-list, small-to-large order", () => {
    expect(SizeOptionModelsAsList).toEqual([
      "Grid_Small",
      "Grid_Medium",
      "Grid_Large",
      "List_Small",
      "List_Medium",
      "List_Large",
    ]);
  });

  it("lists the model order labels in dropdown order", () => {
    expect(Object.entries(MODEL_ORDER_LABELS)).toEqual([
      ["date-asc", "Added (Asc)"],
      ["date-desc", "Added (Desc)"],
      ["name-asc", "Name (A->Z)"],
      ["name-desc", "Name (Z->A)"],
      ["size-asc", "Size (Asc)"],
      ["size-desc", "Size (Desc)"],
      ["modified-asc", "Modified (Asc)"],
      ["modified-desc", "Modified (Desc)"],
    ]);
  });

  it("lists the group order labels in dropdown order without the size options", () => {
    expect(Object.entries(GROUP_ORDER_LABELS)).toEqual([
      ["date-asc", "Added (Asc)"],
      ["date-desc", "Added (Desc)"],
      ["name-asc", "Name (A->Z)"],
      ["name-desc", "Name (Z->A)"],
      ["modified-asc", "Modified (Asc)"],
      ["modified-desc", "Modified (Desc)"],
    ]);
  });
});

describe("convertOrderOptionModelsToEnum", () => {
  it.each<[OrderOptionModels, ModelOrderBy]>([
    ["date-asc", ModelOrderBy.AddedAsc],
    ["date-desc", ModelOrderBy.AddedDesc],
    ["name-asc", ModelOrderBy.NameAsc],
    ["name-desc", ModelOrderBy.NameDesc],
    ["size-asc", ModelOrderBy.SizeAsc],
    ["size-desc", ModelOrderBy.SizeDesc],
    ["modified-asc", ModelOrderBy.ModifiedAsc],
    ["modified-desc", ModelOrderBy.ModifiedDesc],
  ])("maps %s to %s", (orderOption, expected) => {
    expect(convertOrderOptionModelsToEnum(orderOption)).toBe(expected);
  });

  it("covers every model order label", () => {
    const mapped = Object.keys(MODEL_ORDER_LABELS).map((orderOption) =>
      convertOrderOptionModelsToEnum(orderOption as OrderOptionModels),
    );

    expect(new Set(mapped)).toEqual(new Set(Object.values(ModelOrderBy)));
  });

  it.each(["", "unknown", "SizeAsc"])(
    "falls back to AddedDesc for unrecognised value %j",
    (orderOption) => {
      expect(
        convertOrderOptionModelsToEnum(orderOption as OrderOptionModels),
      ).toBe(ModelOrderBy.AddedDesc);
    },
  );
});

describe("convertOrderOptionGroupsToEnum", () => {
  it.each<[OrderOptionGroups, GroupOrderBy]>([
    ["date-asc", GroupOrderBy.CreatedAsc],
    ["date-desc", GroupOrderBy.CreatedDesc],
    ["name-asc", GroupOrderBy.NameAsc],
    ["name-desc", GroupOrderBy.NameDesc],
    ["modified-asc", GroupOrderBy.ModifiedAsc],
    ["modified-desc", GroupOrderBy.ModifiedDesc],
  ])("maps %s to %s", (orderOption, expected) => {
    expect(convertOrderOptionGroupsToEnum(orderOption)).toBe(expected);
  });

  it.each(["", "unknown", "size-asc", "size-desc"])(
    "falls back to CreatedDesc for unrecognised value %j",
    (orderOption) => {
      expect(
        convertOrderOptionGroupsToEnum(orderOption as OrderOptionGroups),
      ).toBe(GroupOrderBy.CreatedDesc);
    },
  );
});
