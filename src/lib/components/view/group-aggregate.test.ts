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
import { createLabelMetaInstance, ILabelApi } from "#lib/api/shared/label_api";
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
vi.mock("$app/navigation", () => ({ goto: vi.fn() }));
vi.mock("$app/paths", () => ({ resolve: (path: string) => path }));
vi.mock("$app/state", () => ({ page: { url: new URL("http://localhost/") } }));
vi.mock("#lib/components/view/three-d-canvas.svelte", () => ({
  default: () => {},
}));
vi.mock("./group-tiny.svelte", async () => ({
  default: (await import("./test/group-aggregate-probe.svelte")).default,
}));
vi.mock("./group-tiny-list.svelte", async () => ({
  default: (await import("./test/group-aggregate-probe.svelte")).default,
}));

afterEach(() => {
  cleanup();
  resetContainer();
  Object.assign(configuration, configurationDefault());
  configurationMeta.applicationReadOnly = false;
});

function setup(
  move = vi.fn().mockResolvedValue(undefined),
  remove = vi.fn().mockResolvedValue(undefined),
  onlyMarked = false,
) {
  const time = "2026-10-24T12:00:00Z";
  const label = createLabelMetaInstance(9, "Blue", 0, time, "blue");
  const model = (id: number, labels: (typeof label)[], flags: string[]) =>
    createModelInstance(
      id,
      `Part ${id}`,
      createBlobInstance(id, `hash-${id}`, "stl.zip", 100, time),
      null,
      null,
      time,
      time,
      null,
      labels,
      flags,
      `model-${id}`,
    );
  const marked = model(1, [label], ["Printed", "Favorite"]);
  const plain = model(2, [], []);
  const destinationModel = model(3, [], []);
  const source = createGroupInstance(
    createGroupMetaInstance(7, "Source", time, time, "source"),
    onlyMarked ? [marked] : [marked, plain],
    [label],
    null,
    ["Printed", "Favorite"],
  );
  const destination = createGroupInstance(
    createGroupMetaInstance(8, "Destination", time, time, "destination"),
    [destinationModel],
    [],
    null,
    [],
  );
  const groups = [source, destination];
  const container = getContainer();
  container.addSingleton(IBlobApi, {
    getBlobThumbnailUrl: vi.fn().mockResolvedValue("data:image/png;base64,"),
  });
  container.addSingleton(IGroupApi, {
    getGroups: vi.fn(async (_filter, page) => (page === 1 ? groups : [])),
    addModelsToGroup: move,
    removeModelsFromGroup: remove,
  });
  container.addSingleton(IModelApi, {});
  container.addSingleton(ILabelApi, {});
  container.addSingleton(IResourceApi, {
    getResources: vi.fn().mockResolvedValue([]),
  });
  container.addSingleton(ISlicerApi, {
    availableSlicers: vi.fn().mockResolvedValue([]),
  });
  configuration.group_split_view = "split-left-right";
  const stream = {
    setSearchText: vi.fn(),
    setOrderBy: vi.fn(),
    setFileTypes: vi.fn(),
    fetch: vi.fn().mockResolvedValueOnce(groups).mockResolvedValue([]),
    getAll: vi.fn().mockResolvedValue(groups),
  };
  render(GroupGrid, { groupStream: stream as IGroupStreamManager });
  return { move, remove };
}

async function selectMarkedModel() {
  await fireEvent.click(await screen.findByRole("option", { name: /Source/ }));
  const model = await screen.findByRole("option", { name: /Part 1/ });
  await fireEvent.click(model);
}

function aggregate(id: number) {
  return JSON.parse(screen.getByTestId(`aggregate-${id}`).textContent!);
}

it("recomputes source and destination aggregate flags and labels after moving a model", async () => {
  const { move } = setup();
  await selectMarkedModel();
  await fireEvent.click(screen.getByRole("button", { name: "Move to group" }));
  await fireEvent.click(
    await screen.findByRole("button", { name: "Destination" }),
  );
  await waitFor(() => expect(move).toHaveBeenCalledTimes(1));
  await waitFor(() =>
    expect(aggregate(7)).toEqual({
      flags: { printed: false, favorite: false },
      labels: [],
    }),
  );
  expect(aggregate(8)).toEqual({
    flags: { printed: true, favorite: true },
    labels: ["Blue"],
  });
});

it("recomputes the group's flags and labels after ungrouping its marked model", async () => {
  const { remove } = setup();
  await selectMarkedModel();
  await fireEvent.click(screen.getByRole("button", { name: /Edit model/i }));
  await fireEvent.click(screen.getByRole("button", { name: /Model actions/i }));
  await fireEvent.click(
    await screen.findByRole("menuitem", { name: /Remove from current group/i }),
  );
  await waitFor(() => expect(remove).toHaveBeenCalledTimes(1));
  await waitFor(() =>
    expect(aggregate(7)).toEqual({
      flags: { printed: false, favorite: false },
      labels: [],
    }),
  );
});

it("adds the moved model's flags and labels to the destination aggregate", async () => {
  const { move } = setup();
  await selectMarkedModel();
  await fireEvent.click(screen.getByRole("button", { name: "Move to group" }));
  await fireEvent.click(
    await screen.findByRole("button", { name: "Destination" }),
  );
  await waitFor(() => expect(move).toHaveBeenCalledTimes(1));
  await waitFor(() =>
    expect(aggregate(8)).toEqual({
      flags: { printed: true, favorite: true },
      labels: ["Blue"],
    }),
  );
});

it("clears aggregate flags and labels when its last model is moved away", async () => {
  const { move } = setup(undefined, undefined, true);
  await selectMarkedModel();
  await fireEvent.click(screen.getByRole("button", { name: "Move to group" }));
  await fireEvent.click(
    await screen.findByRole("button", { name: "Destination" }),
  );
  await waitFor(() => expect(move).toHaveBeenCalledTimes(1));
  await waitFor(() =>
    expect(aggregate(7)).toEqual({
      flags: { printed: false, favorite: false },
      labels: [],
    }),
  );
});

it("leaves both aggregate flags and labels unchanged when a move fails", async () => {
  const move = vi.fn().mockRejectedValue(new Error("offline"));
  vi.spyOn(console, "error").mockImplementation(() => {});
  setup(move);
  await selectMarkedModel();
  await fireEvent.click(screen.getByRole("button", { name: "Move to group" }));
  await fireEvent.click(
    await screen.findByRole("button", { name: "Destination" }),
  );
  expect(
    await screen.findByText("Could not move models. Try again."),
  ).toBeTruthy();
  expect(aggregate(7)).toEqual({
    flags: { printed: true, favorite: true },
    labels: ["Blue"],
  });
  expect(aggregate(8)).toEqual({
    flags: { printed: false, favorite: false },
    labels: [],
  });
});
