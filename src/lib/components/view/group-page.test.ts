// @vitest-environment jsdom
import { cleanup, fireEvent, render, waitFor } from "@testing-library/svelte";
import { afterEach, expect, it, vi } from "vitest";
import { getContainer, resetContainer } from "#lib/api/dependency_injection";
import {
  createGroupInstance,
  createGroupMetaInstance,
  IGroupApi,
  type Group,
  type GroupFilter,
} from "#lib/api/shared/group_api";
import { IModelApi } from "#lib/api/shared/model_api";
import { ILabelApi } from "#lib/api/shared/label_api";
import { IResourceApi } from "#lib/api/shared/resource_api";
import GroupPage from "./group-page.svelte";

vi.hoisted(() => {
  window.matchMedia ??= (query: string) =>
    ({
      matches: false,
      media: query,
      addEventListener() {},
      removeEventListener() {},
    }) as unknown as MediaQueryList;
});

vi.mock("$app/navigation", () => ({ goto: vi.fn() }));
vi.mock("$app/paths", () => ({ resolve: (path: string) => path }));
vi.mock("$app/state", () => ({ page: { url: new URL("http://localhost/") } }));

afterEach(() => {
  cleanup();
  resetContainer();
});

function group(id: number, name: string): Group {
  return createGroupInstance(
    createGroupMetaInstance(
      id,
      name,
      "2026-01-01",
      "2026-01-01",
      `group-${id}`,
    ),
    [],
    [],
    null,
    [],
  );
}

function provideApis(getGroups: (filter: GroupFilter) => Promise<Group[]>) {
  const container = getContainer();
  container.addSingleton(IGroupApi, { getGroups });
  container.addSingleton(IModelApi, {});
  container.addSingleton(ILabelApi, {});
  container.addSingleton(IResourceApi, {
    getResources: vi.fn().mockResolvedValue([]),
  });
}

it("shows group not found when the requested group is absent", async () => {
  provideApis(async () => []);
  const page = render(GroupPage, { group: { id: 99 } });

  expect(await page.findByText("Group not found")).toBeTruthy();
});

it("allows retrying a failed group load", async () => {
  const error = vi.spyOn(console, "error").mockImplementation(() => {});
  const getGroups = vi
    .fn()
    .mockRejectedValueOnce(new Error("network failure"))
    .mockResolvedValueOnce([group(1, "First")]);
  provideApis(getGroups);
  const page = render(GroupPage, { group: { id: 1 } });

  try {
    const retry = await page.findByRole("button", { name: /retry/i });
    expect(page.queryByText("Group not found")).toBeNull();
    await fireEvent.click(retry);
    expect(await page.findByText("Group 'First'")).toBeTruthy();
    expect(getGroups).toHaveBeenCalledTimes(2);
  } finally {
    error.mockRestore();
  }
});

it("loads the group for a changed group id", async () => {
  const groups = [group(1, "First"), group(2, "Second")];
  provideApis(async (filter) =>
    groups.filter((g) => filter.groupIds?.includes(g.meta.id)),
  );
  const page = render(GroupPage, { group: { id: 1 } });

  expect(await page.findByText("Group 'First'")).toBeTruthy();
  await page.rerender({ group: { id: 2 } });

  expect(await page.findByText("Group 'Second'")).toBeTruthy();
  expect(page.queryByText("Group 'First'")).toBeNull();
});

it("does not replace the current group when an earlier request resolves late", async () => {
  const pending = new Map<number, (groups: Group[]) => void>();
  provideApis(
    (filter) =>
      new Promise((resolve) => pending.set(filter.groupIds![0], resolve)),
  );
  const page = render(GroupPage, { group: { id: 1 } });
  await waitFor(() => expect(pending.has(1)).toBe(true));
  await page.rerender({ group: { id: 2 } });
  await waitFor(() => expect(pending.has(2)).toBe(true));

  pending.get(2)!([group(2, "Second")]);
  expect(await page.findByText("Group 'Second'")).toBeTruthy();
  pending.get(1)!([group(1, "First")]);
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(page.queryByText("Group 'First'")).toBeNull();
  expect(page.getByText("Group 'Second'")).toBeTruthy();
});
