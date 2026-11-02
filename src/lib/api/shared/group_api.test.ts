/**
 * Tests for getGroupsOfModels: an empty id list makes no request, and a longer
 * one drains every page at the largest page size the server accepts, since a
 * page size above it is answered with a 400. A page shorter than that is the
 * last, so draining stops there without asking for an empty page.
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

  const filterOf = (modelIds: number[]) =>
    defaultGroupFilter({
      modelIds,
      orderBy: GroupOrderBy.CreatedAsc,
      includeUngroupedModels: true,
    });

  async function drain(pages: Group[][], modelIds: number[]) {
    const getGroups = vi.fn(
      async (_filter: GroupFilter, page: number) => pages[page - 1] ?? [],
    );
    const groups = await getGroupsOfModels(
      { getGroups } as unknown as IGroupApi,
      modelIds,
      GroupOrderBy.CreatedAsc,
    );

    return { groups, calls: getGroups.mock.calls };
  }

  it("returns the groups of every full page and the short one after it, each fetched at the max page size", async () => {
    const fullPage = Array.from({ length: MAX_PAGE_SIZE }, (_, i) =>
      fakeGroup(i),
    );
    const modelIds = Array.from({ length: MAX_PAGE_SIZE + 1 }, (_, i) => i);

    const { groups, calls } = await drain(
      [fullPage, [fakeGroup(MAX_PAGE_SIZE)]],
      modelIds,
    );

    expect(groups.map((group) => group.meta.id)).toEqual(modelIds);
    expect(calls).toEqual([
      [filterOf(modelIds), 1, MAX_PAGE_SIZE],
      [filterOf(modelIds), 2, MAX_PAGE_SIZE],
    ]);
  });

  it("makes one request when the first page is short", async () => {
    const { groups, calls } = await drain(
      [[fakeGroup(1), fakeGroup(2)]],
      [1, 2],
    );

    expect(groups.map((group) => group.meta.id)).toEqual([1, 2]);
    expect(calls).toEqual([[filterOf([1, 2]), 1, MAX_PAGE_SIZE]]);
  });
});
