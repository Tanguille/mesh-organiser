// @vitest-environment jsdom
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/svelte";
import { afterEach, expect, it, vi } from "vitest";
import { getContainer, resetContainer } from "#lib/api/dependency_injection";
import { IBlobApi, createBlobInstance } from "#lib/api/shared/blob_api";
import {
  IGroupApi,
  createGroupInstance,
  createGroupMetaInstance,
  type IGroupStreamManager,
} from "#lib/api/shared/group_api";
import { IModelApi, createModelInstance } from "#lib/api/shared/model_api";
import { ILabelApi } from "#lib/api/shared/label_api";
import { IResourceApi } from "#lib/api/shared/resource_api";
import { ISlicerApi } from "#lib/api/shared/slicer_api";
import { configurationDefault } from "#lib/api/shared/settings_api";
import { configuration, configurationMeta } from "#lib/configuration.svelte";
import GroupGrid from "./group-grid.svelte";

vi.hoisted(() => {
  window.matchMedia = (query: string) =>
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
vi.mock("#lib/components/view/three-d-canvas.svelte", () => ({
  default: () => {},
}));

afterEach(() => {
  cleanup();
  resetContainer();
  Object.assign(configuration, configurationDefault());
  configurationMeta.applicationReadOnly = false;
});

async function setup(list = false) {
  const date = "2026-10-24T12:00:00Z";
  const groups = [-1, -2, 3].map((groupId, index) => {
    const id = index + 1;
    const meta = createGroupMetaInstance(
      groupId,
      `Card ${id}`,
      date,
      date,
      `group-${id}`,
    );
    const model = createModelInstance(
      id,
      `Part ${id}`,
      createBlobInstance(id, `hash-${id}`, "stl.zip", 100, date),
      null,
      null,
      date,
      date,
      groupId < 0 ? null : meta,
      [],
      [],
      `model-${id}`,
    );
    return createGroupInstance(meta, [model], [], null, []);
  });
  const deleteModels = vi.fn().mockResolvedValue(undefined);
  const container = getContainer();
  container.addSingleton(IBlobApi, {
    getBlobThumbnailUrl: vi.fn().mockResolvedValue("data:image/png;base64,"),
  });
  container.addSingleton(IGroupApi, {});
  container.addSingleton(IModelApi, { deleteModels });
  container.addSingleton(ILabelApi, {});
  container.addSingleton(IResourceApi, {
    getResources: vi.fn().mockResolvedValue([]),
  });
  container.addSingleton(ISlicerApi, {
    availableSlicers: vi.fn().mockResolvedValue([]),
  });
  configuration.group_split_view = "no_split";
  if (list) configuration.size_option_groups = "List_Medium";
  const stream = {
    setSearchText: vi.fn(),
    setOrderBy: vi.fn(),
    setFileTypes: vi.fn(),
    fetch: vi.fn().mockResolvedValueOnce(groups).mockResolvedValue([]),
    getAll: vi.fn().mockResolvedValue(groups),
  };
  render(GroupGrid, { groupStream: stream as IGroupStreamManager });
  const first = await screen.findByRole("option", { name: /Card 1/ });
  const second = screen.getByRole("option", { name: /Card 2/ });
  const real = screen.getByRole("option", { name: /Card 3/ });
  for (const option of [first, second, real]) option.scrollIntoView = vi.fn();
  return { first, second, real, groups, deleteModels };
}

it("moves multiple selected ungrouped models to trash with Delete", async () => {
  const { first, second, groups, deleteModels } = await setup();
  await fireEvent.click(first);
  await fireEvent.click(second, { ctrlKey: true });
  await fireEvent.keyDown(second, { key: "Delete" });
  await waitFor(() => expect(deleteModels).toHaveBeenCalledOnce());
  expect(
    deleteModels.mock.calls[0][0].map((model: { id: number }) => model.id),
  ).toEqual(groups.slice(0, 2).map((group) => group.models[0].id));
});

it("offers trash for an ungrouped model in the group context menu", async () => {
  const { first, groups, deleteModels } = await setup();
  await fireEvent.contextMenu(first);
  await fireEvent.click(
    await screen.findByRole("menuitem", { name: /move to trash/i }),
  );
  await waitFor(() =>
    expect(deleteModels).toHaveBeenCalledExactlyOnceWith([groups[0].models[0]]),
  );
});

it("does not trash models for real or mixed groups or while typing", async () => {
  const { first, real, deleteModels } = await setup();
  await fireEvent.click(real);
  await fireEvent.keyDown(real, { key: "Delete" });
  await fireEvent.click(first, { ctrlKey: true });
  await fireEvent.keyDown(first, { key: "Delete" });
  const search = screen.getByRole("textbox", { name: "Search" });
  await fireEvent.keyDown(search, { key: "Delete" });
  expect(deleteModels).not.toHaveBeenCalled();
});

it("supports Delete on an ungrouped model in list view", async () => {
  const { first, groups, deleteModels } = await setup(true);
  await fireEvent.click(first);
  await fireEvent.keyDown(first, { key: "Delete" });
  await waitFor(() =>
    expect(deleteModels).toHaveBeenCalledExactlyOnceWith([groups[0].models[0]]),
  );
});

it("does not offer trash or handle Delete in read-only mode", async () => {
  configurationMeta.applicationReadOnly = true;
  const { first, deleteModels } = await setup();
  await fireEvent.click(first);
  await fireEvent.keyDown(first, { key: "Delete" });
  await fireEvent.contextMenu(first);
  expect(screen.queryByRole("menuitem", { name: /move to trash/i })).toBeNull();
  expect(deleteModels).not.toHaveBeenCalled();
});

it("keeps a real group card when its last model is trashed but drops an ungrouped card", async () => {
  const { first, real } = await setup();
  await fireEvent.click(real);
  await fireEvent.click(
    await screen.findByRole("button", { name: /move to trash/i }),
  );
  await waitFor(() =>
    expect(screen.queryByRole("heading", { name: "Part 3" })).toBeNull(),
  );
  expect(screen.queryByRole("option", { name: /Card 3/ })).not.toBeNull();

  await fireEvent.click(first);
  await fireEvent.click(
    await screen.findByRole("button", { name: /move to trash/i }),
  );
  await waitFor(() =>
    expect(screen.queryByRole("option", { name: /Card 1/ })).toBeNull(),
  );
});
