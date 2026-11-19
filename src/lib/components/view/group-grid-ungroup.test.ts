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
import { configuration } from "#lib/configuration.svelte";
import GroupGrid from "./group-grid.svelte";
import ModelGrid from "./model-grid.svelte";
import type { IModelStreamManager } from "#lib/api/shared/model_api";

vi.hoisted(() => {
  window.matchMedia = (query: string) =>
    ({
      matches: false,
      media: query,
      addEventListener() {},
      removeEventListener() {},
    }) as unknown as MediaQueryList;
});

it("does not report a group as empty when its last loaded model is deleted but another remains", async () => {
  const timestamp = "2026-10-24T12:00:00Z";
  const meta = createGroupMetaInstance(
    7,
    "Parts",
    timestamp,
    timestamp,
    "group-7",
  );
  const models = [1, 2].map((id) =>
    createModelInstance(
      id,
      `Part ${id}`,
      createBlobInstance(id, `hash-${id}`, "stl.zip", 1024, timestamp),
      null,
      null,
      timestamp,
      timestamp,
      meta,
      [],
      [],
      `model-${id}`,
    ),
  );
  const container = getContainer();
  container.addSingleton(IBlobApi, {
    getBlobThumbnailUrl: vi.fn().mockResolvedValue("data:image/png;base64,"),
  });
  container.addSingleton(IGroupApi, {});
  const deleteModels = vi.fn().mockResolvedValue(undefined);
  container.addSingleton(IModelApi, { deleteModels });
  container.addSingleton(ILabelApi, {});
  container.addSingleton(IResourceApi, {
    getResources: vi.fn().mockResolvedValue([]),
  });
  container.addSingleton(ISlicerApi, {
    availableSlicers: vi.fn().mockResolvedValue([]),
  });
  const stream = {
    setSearchText: vi.fn(),
    setOrderBy: vi.fn(),
    setFileTypes: vi.fn(),
    fetch: vi
      .fn()
      .mockResolvedValueOnce([models[0]])
      .mockResolvedValueOnce([models[1]])
      .mockResolvedValue([]),
    getAll: vi.fn().mockResolvedValue(models),
  };
  const onEmpty = vi.fn();
  const view = render(ModelGrid, {
    modelStream: stream as IModelStreamManager,
    initialEditMode: true,
    onEmpty,
  });
  await screen.findByRole("option", { name: /Part 1/ });
  await fireEvent.click(
    view.container.querySelector(".lucide-ellipsis")!.closest("button")!,
  );
  await fireEvent.click(await screen.findByText("Move to trash"));
  await waitFor(() => expect(deleteModels).toHaveBeenCalledWith([models[0]]));
  await waitFor(() =>
    expect(screen.queryByRole("option", { name: /Part 1/ })).toBeNull(),
  );
  expect(onEmpty).not.toHaveBeenCalled();
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
});

it("keeps the remaining group members together when selected models are ungrouped", async () => {
  const timestamp = "2026-10-24T12:00:00Z";
  const meta = createGroupMetaInstance(
    7,
    "Calibration parts",
    timestamp,
    timestamp,
    "group-7",
  );
  const models = [1, 2, 3].map((id) =>
    createModelInstance(
      id,
      `Part ${id}`,
      createBlobInstance(id, `hash-${id}`, "stl.zip", 1024, timestamp),
      null,
      null,
      timestamp,
      timestamp,
      meta,
      [],
      [],
      `model-${id}`,
    ),
  );
  const removeModelsFromGroup = vi.fn().mockResolvedValue(undefined);
  const container = getContainer();
  container.addSingleton(IBlobApi, {
    getBlobThumbnailUrl: vi.fn().mockResolvedValue("data:image/png;base64,"),
  });
  container.addSingleton(IGroupApi, { removeModelsFromGroup });
  container.addSingleton(IModelApi, {});
  container.addSingleton(ILabelApi, {});
  container.addSingleton(IResourceApi, {
    getResources: vi.fn().mockResolvedValue([]),
  });
  container.addSingleton(ISlicerApi, {
    availableSlicers: vi.fn().mockResolvedValue([]),
  });
  configuration.group_split_view = "split-left-right";
  const group = createGroupInstance(meta, models, [], null, []);
  const stream = {
    setSearchText: vi.fn(),
    setOrderBy: vi.fn(),
    setFileTypes: vi.fn(),
    fetch: vi.fn().mockResolvedValueOnce([group]).mockResolvedValue([]),
    getAll: vi.fn().mockResolvedValue([group]),
  };
  render(GroupGrid, { groupStream: stream as IGroupStreamManager });

  const groupOption = await screen.findByRole("option", {
    name: /Calibration parts/,
  });
  groupOption.scrollIntoView = vi.fn();
  await fireEvent.click(groupOption);
  const first = (
    await screen.findByRole("heading", { name: "Part 1" })
  ).closest('[role="option"]')!;
  const second = screen
    .getByRole("heading", { name: "Part 2" })
    .closest('[role="option"]')!;
  first.scrollIntoView = vi.fn();
  await fireEvent.click(first);
  await fireEvent.click(second, { ctrlKey: true });
  await fireEvent.click(
    screen.getByRole("button", { name: /Remove from group/ }),
  );

  await waitFor(() =>
    expect(removeModelsFromGroup).toHaveBeenCalledWith(models.slice(0, 2)),
  );
  expect(
    screen.getByRole("option", { name: /Calibration parts/ }),
  ).toBeTruthy();
  expect(
    screen
      .getByRole("heading", { name: "Part 3", level: 2 })
      .closest('[role="option"]'),
  ).toBeTruthy();
  expect(
    screen.getByRole("heading", { name: "Part 1" }).closest('[role="option"]'),
  ).toBeTruthy();
  expect(
    screen.getByRole("heading", { name: "Part 2" }).closest('[role="option"]'),
  ).toBeTruthy();
});
