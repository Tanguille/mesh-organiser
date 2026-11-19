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
import { createBlobInstance, IBlobApi } from "#lib/api/shared/blob_api";
import {
  createGroupInstance,
  createGroupMetaInstance,
  IGroupApi,
  type IGroupStreamManager,
} from "#lib/api/shared/group_api";
import { ILabelApi } from "#lib/api/shared/label_api";
import { createModelInstance, IModelApi } from "#lib/api/shared/model_api";
import { IResourceApi } from "#lib/api/shared/resource_api";
import { ISlicerApi } from "#lib/api/shared/slicer_api";
import { configuration, configurationMeta } from "#lib/configuration.svelte";
import { configurationDefault } from "#lib/api/shared/settings_api";

import GroupGrid from "./group-grid.svelte";

vi.hoisted(() => {
  window.matchMedia = (query: string) =>
    ({
      matches: false,
      media: query,
      addEventListener() {},
      removeEventListener() {},
    }) as unknown as MediaQueryList;
  HTMLElement.prototype.scrollIntoView = () => {};
});
const navigation = vi.hoisted(() => ({ goto: vi.fn() }));
vi.mock("$app/navigation", () => navigation);
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
function setup() {
  const container = getContainer();
  container.addSingleton(IBlobApi, {
    getBlobThumbnailUrl: vi.fn().mockResolvedValue("data:image/png;base64,"),
  });
  container.addSingleton(IGroupApi, {
    getGroups: vi.fn().mockResolvedValue([]),
  });
  container.addSingleton(IModelApi, {});
  container.addSingleton(ILabelApi, {});
  container.addSingleton(IResourceApi, {
    getResources: vi.fn().mockResolvedValue([]),
  });
  container.addSingleton(ISlicerApi, {
    availableSlicers: vi.fn().mockResolvedValue([]),
  });
  const date = "2026-10-24T12:00:00Z";
  const models = [1, 2].map((id) =>
    createModelInstance(
      id,
      `Part ${id}`,
      createBlobInstance(id, `hash-${id}`, "stl.zip", 1024, date),
      null,
      null,
      date,
      date,
      null,
      [],
      [],
      `model-${id}`,
    ),
  );
  const group = createGroupInstance(
    createGroupMetaInstance(7, "Calibration", date, date, "group-7"),
    models,
    [],
    null,
    [],
  );
  const stream = (items: unknown[]) => ({
    setSearchText: vi.fn(),
    setOrderBy: vi.fn(),
    setFileTypes: vi.fn(),
    fetch: vi.fn().mockResolvedValueOnce(items).mockResolvedValue([]),
    getAll: vi.fn().mockResolvedValue(items),
  });
  return { models, group, stream };
}

it("updates source and destination after a pending move even when selection changes", async () => {
  const { group, stream } = setup();
  configuration.group_split_view = "split-left-right";
  const targetModel = createModelInstance(
    3,
    "Part 3",
    createBlobInstance(3, "hash-3", "stl.zip", 1024, "2026-10-24"),
    null,
    null,
    "2026-10-24",
    "2026-10-24",
    null,
    [],
    [],
    "model-3",
  );
  const destination = createGroupInstance(
    createGroupMetaInstance(
      8,
      "Destination",
      "2026-10-24",
      "2026-10-24",
      "group-8",
    ),
    [targetModel],
    [],
    null,
    [],
  );
  let finish!: () => void;
  const addModelsToGroup = vi.fn().mockImplementation(
    () =>
      new Promise<void>((resolve) => {
        finish = resolve;
      }),
  );
  getContainer().addSingleton(IGroupApi, {
    getGroups: vi.fn().mockResolvedValue([group, destination]),
    addModelsToGroup,
  });
  render(GroupGrid, {
    groupStream: stream([group, destination]) as IGroupStreamManager,
  });
  const sourceOption = await screen.findByRole("option", {
    name: /Calibration/,
  });
  const destinationOption = await screen.findByRole("option", {
    name: /Destination/,
  });
  await fireEvent.click(sourceOption);
  const first = (
    await screen.findByRole("heading", { name: "Part 1" })
  ).closest('[role="option"]')!;
  await fireEvent.click(first);
  await fireEvent.click(screen.getByRole("button", { name: /move to group/i }));
  await fireEvent.click(
    await screen.findByRole("button", { name: "Destination" }),
  );
  await waitFor(() => expect(addModelsToGroup).toHaveBeenCalled());
  await fireEvent.click(destinationOption);
  finish();
  await waitFor(() =>
    expect(destinationOption.getAttribute("aria-selected")).toBe("true"),
  );
  await waitFor(() =>
    expect(
      screen.queryByRole("heading", { name: "Part 1", level: 2 }),
    ).toBeTruthy(),
  );
  expect(
    screen.getByRole("heading", { name: "Part 3", level: 2 }),
  ).toBeTruthy();
  await fireEvent.click(sourceOption);
  expect(
    screen.queryByRole("heading", { name: "Part 1", level: 2 }),
  ).toBeNull();
  expect(
    screen.getByRole("heading", { name: "Part 2", level: 2 }),
  ).toBeTruthy();
});

it.each(["Enter", "double-click"])(
  "opens a group through %s activation",
  async (action) => {
    const { group, stream } = setup();
    navigation.goto.mockClear();
    render(GroupGrid, { groupStream: stream([group]) as IGroupStreamManager });
    const option = await screen.findByRole("option", { name: /Calibration/ });
    if (action === "Enter") await fireEvent.keyDown(option, { key: "Enter" });
    else await fireEvent.dblClick(option);
    expect(navigation.goto).toHaveBeenCalledWith("/group/7");
  },
);

it("keeps the source group visible after moving its last model", async () => {
  const { group, stream } = setup();
  group.models = group.models.slice(0, 1);
  const date = "2026-10-24";
  const other = createModelInstance(
    3,
    "Part 3",
    createBlobInstance(3, "hash-3", "stl.zip", 1024, date),
    null,
    null,
    date,
    date,
    null,
    [],
    [],
    "model-3",
  );
  const destination = createGroupInstance(
    createGroupMetaInstance(8, "Destination", date, date, "group-8"),
    [other],
    [],
    null,
    [],
  );
  const addModelsToGroup = vi.fn().mockResolvedValue(undefined);
  getContainer().addSingleton(IGroupApi, {
    getGroups: vi.fn().mockResolvedValue([group, destination]),
    addModelsToGroup,
  });
  render(GroupGrid, {
    groupStream: stream([group, destination]) as IGroupStreamManager,
  });
  const source = await screen.findByRole("option", { name: /Calibration/ });
  await fireEvent.click(source);
  await fireEvent.click(screen.getByRole("button", { name: /move to group/i }));
  await fireEvent.click(
    await screen.findByRole("button", { name: "Destination" }),
  );
  await waitFor(() => expect(addModelsToGroup).toHaveBeenCalled());
  await waitFor(() =>
    expect(
      screen.queryByRole("option", { name: /Calibration/ }),
    ).not.toBeNull(),
  );
});
