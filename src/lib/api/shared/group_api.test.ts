/**
 * Tests for getGroupsOfModels: an empty id list makes no request, and a longer
 * one drains every page at the largest page size the server accepts, since a
 * page size above it is answered with a 400.
 */

import { describe, expect, it, vi } from "vitest";
import {
  defaultGroupFilter,
  getGroupsOfModels,
  GroupOrderBy,
  type Group,
  type GroupFilter,
  type IGroupApi,
} from "./group_api";
import { MAX_PAGE_SIZE } from "./model_api";

function fakeGroup(id: number): Group {
  return { meta: { id } } as Group;
}

describe("getGroupsOfModels", () => {
  it("returns no groups without a request for an empty id list", async () => {
    const getGroups = vi.fn();

    const groups = await getGroupsOfModels(
      { getGroups } as unknown as IGroupApi,
      [],
      GroupOrderBy.NameDesc,
    );

    expect(groups).toEqual([]);
    expect(getGroups).not.toHaveBeenCalled();
  });

  it("returns the groups of every page, each fetched at the max page size", async () => {
    const pages = [[fakeGroup(1), fakeGroup(2)], [fakeGroup(3)], []];
    const getGroups = vi.fn(
      async (_filter: GroupFilter, page: number) => pages[page - 1] ?? [],
    );
    const modelIds = Array.from({ length: MAX_PAGE_SIZE + 1 }, (_, i) => i);

    const groups = await getGroupsOfModels(
      { getGroups } as unknown as IGroupApi,
      modelIds,
      GroupOrderBy.CreatedAsc,
    );

    expect(groups.map((group) => group.meta.id)).toEqual([1, 2, 3]);
    const filter = defaultGroupFilter({
      modelIds,
      orderBy: GroupOrderBy.CreatedAsc,
      includeUngroupedModels: true,
    });
    expect(getGroups.mock.calls).toEqual([
      [filter, 1, MAX_PAGE_SIZE],
      [filter, 2, MAX_PAGE_SIZE],
      [filter, 3, MAX_PAGE_SIZE],
    ]);
  });
});
