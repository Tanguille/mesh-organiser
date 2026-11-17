// @vitest-environment jsdom
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { getContainer, resetContainer } from "#lib/api/dependency_injection";
import { IBlobApi, createBlobInstance } from "#lib/api/shared/blob_api";
import {
  IModelApi,
  PredefinedModelStreamManager,
  createModelInstance,
  type IModelStreamManager,
} from "#lib/api/shared/model_api";
import {
  IGroupApi,
  createGroupInstance,
  createGroupMetaInstance,
  type IGroupStreamManager,
} from "#lib/api/shared/group_api";
import { ILabelApi } from "#lib/api/shared/label_api";
import { IResourceApi } from "#lib/api/shared/resource_api";
import { ISlicerApi } from "#lib/api/shared/slicer_api";
import { configurationDefault } from "#lib/api/shared/settings_api";
import { configuration } from "#lib/configuration.svelte";
import ModelGrid from "./model-grid.svelte";
import GroupGrid from "./group-grid.svelte";

// jsdom has no matchMedia; svelte/reactivity touches it at import time and
// IsMobile / group-grid's split-view MediaQuery read it. Default to a wide desktop.
const media = vi.hoisted(() => {
  const state = { width: 1920 };
  window.matchMedia = (query: string) =>
    ({
      get matches() {
        const maxWidth = /max-width:\s*(\d+)px/.exec(query);

        return maxWidth !== null && state.width <= Number(maxWidth[1]);
      },
      media: query,
      addEventListener: () => {},
      removeEventListener: () => {},
    }) as unknown as MediaQueryList;

  return state;
});

vi.mock("$app/navigation", () => ({ goto: vi.fn() }));
vi.mock("$app/paths", () => ({ resolve: (path: string) => path }));
vi.mock("$app/state", () => ({ page: { url: new URL("http://localhost/") } }));
vi.mock("#lib/components/view/three-d-canvas.svelte", () => ({
  default: () => {},
}));

afterEach(() => {
  // Unmount first: a still-mounted grid re-renders on the configuration reset
  // and reads the apis from the emptied container.
  cleanup();
  media.width = 1920;
  resetContainer();
  Object.assign(configuration, configurationDefault());
});

// Empty streams: nothing loaded, so the right pane shows the empty state.
function emptyStream() {
  return {
    setSearchText: vi.fn(),
    setOrderBy: vi.fn(),
    setFileTypes: vi.fn(),
    fetch: vi.fn().mockResolvedValue([]),
    getAll: vi.fn().mockResolvedValue([]),
  };
}

const grids = [
  {
    name: "ModelGrid",
    emptyText: "No model selected",
    mount: (stream: ReturnType<typeof emptyStream>) =>
      render(ModelGrid, { modelStream: stream as IModelStreamManager }),
  },
  {
    name: "GroupGrid",
    emptyText: "No group selected",
    mount: (stream: ReturnType<typeof emptyStream>) =>
      render(GroupGrid, { groupStream: stream as IGroupStreamManager }),
  },
];

describe.each(grids)("$name", ({ emptyText, mount }) => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("renders the dashed empty state when nothing is selected", async () => {
    const stream = emptyStream();
    mount(stream);

    const empty = screen.getByText(emptyText);
    expect(empty.parentElement?.className).toContain("border-dashed");
    await waitFor(() => expect(stream.fetch).toHaveBeenCalled());
  });

  it("applies search text only after the 200ms debounce", async () => {
    const stream = emptyStream();
    mount(stream);
    await waitFor(() => expect(stream.fetch).toHaveBeenCalledTimes(1));

    vi.useFakeTimers();
    const input = screen.getByPlaceholderText("Search");
    await fireEvent.input(input, { target: { value: "bo" } });
    await fireEvent.input(input, { target: { value: "  bolt  " } });

    vi.advanceTimersByTime(199);
    expect(stream.setSearchText).not.toHaveBeenCalled();

    vi.advanceTimersByTime(1);
    // Debounced: only the last value, trimmed.
    expect(stream.setSearchText).toHaveBeenCalledExactlyOnceWith("bolt");
    vi.useRealTimers();
    await waitFor(() => expect(stream.fetch).toHaveBeenCalledTimes(2));
  });

  it("clears the search filter (null) for whitespace-only input", async () => {
    const stream = emptyStream();
    mount(stream);
    await waitFor(() => expect(stream.fetch).toHaveBeenCalled());

    vi.useFakeTimers();
    await fireEvent.input(screen.getByPlaceholderText("Search"), {
      target: { value: "   " },
    });
    vi.advanceTimersByTime(200);

    expect(stream.setSearchText).toHaveBeenCalledExactlyOnceWith(null);
  });
});

// Registers the apis the grids' items read from and returns two models of one group.
function calibrationParts() {
  const container = getContainer();
  container.addSingleton(IBlobApi, {
    getBlobThumbnailUrl: vi.fn().mockResolvedValue("data:image/png;base64,"),
  });
  container.addSingleton(IGroupApi, {});
  container.addSingleton(IModelApi, {});
  container.addSingleton(ILabelApi, {});
  container.addSingleton(IResourceApi, {
    getResources: vi.fn().mockResolvedValue([]),
  });
  container.addSingleton(ISlicerApi, {
    availableSlicers: vi.fn().mockResolvedValue([]),
  });
  const timestamp = "2026-10-24T12:00:00Z";
  const meta = createGroupMetaInstance(
    7,
    "Calibration parts",
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

  return { meta, models };
}

function groupStreamWithModels() {
  const { meta, models } = calibrationParts();
  const group = createGroupInstance(meta, models, [], null, []);
  const stream = emptyStream();
  stream.fetch.mockResolvedValueOnce([group]);

  return stream;
}

it("clears the selected model when search hides it", async () => {
  const { models } = calibrationParts();
  const stream = emptyStream();
  stream.fetch.mockResolvedValueOnce(models).mockResolvedValueOnce([models[1]]);
  stream.getAll.mockResolvedValue(models);
  render(ModelGrid, { modelStream: stream as IModelStreamManager });

  const first = await screen.findByRole("option", { name: /Part 1/ });
  first.scrollIntoView = vi.fn();
  await fireEvent.click(first);
  expect(first.getAttribute("aria-selected")).toBe("true");

  await fireEvent.input(screen.getByPlaceholderText("Search"), {
    target: { value: "Part 2" },
  });
  await waitFor(() =>
    expect(stream.setSearchText).toHaveBeenCalledWith("Part 2"),
  );
  await waitFor(() =>
    expect(screen.queryByRole("option", { name: /Part 1/ })).toBeNull(),
  );
  expect(screen.queryByRole("heading", { name: "Part 1" })).toBeNull();
});

it("removes the model whose deletion completed, preserving a newer selection", async () => {
  const { models } = calibrationParts();
  let finishDelete!: () => void;
  const deleteModels = vi.fn(
    () => new Promise<void>((resolve) => (finishDelete = resolve)),
  );
  getContainer().addSingleton(IModelApi, { deleteModels });
  const stream = emptyStream();
  stream.fetch.mockResolvedValueOnce(models);
  stream.getAll.mockResolvedValue(models);
  const { container } = render(ModelGrid, {
    modelStream: stream as IModelStreamManager,
    initialEditMode: true,
  });

  const first = await screen.findByRole("option", { name: /Part 1/ });
  first.scrollIntoView = vi.fn();
  await fireEvent.click(first);
  const menuButton = container
    .querySelector(".lucide-ellipsis")!
    .closest("button")!;
  await fireEvent.click(menuButton);
  await fireEvent.click(await screen.findByText("Move to trash"));
  expect(deleteModels).toHaveBeenCalledWith([models[0]]);

  const second = screen.getByRole("option", { name: /Part 2/ });
  second.scrollIntoView = vi.fn();
  await fireEvent.click(second);
  finishDelete();

  await waitFor(() =>
    expect(screen.queryByRole("option", { name: /Part 1/ })).toBeNull(),
  );
  expect(
    screen
      .getByRole("option", { name: /Part 2/ })
      .getAttribute("aria-selected"),
  ).toBe("true");
});

it("does not reload a deleted model when a predefined stream is recreated from its source", async () => {
  const { models } = calibrationParts();
  getContainer().addSingleton(IModelApi, {
    deleteModels: vi.fn().mockResolvedValue(undefined),
  });
  const { container, rerender } = render(ModelGrid, {
    modelStream: new PredefinedModelStreamManager(models),
    initialEditMode: true,
  });
  const first = await screen.findByRole("option", { name: /Part 1/ });
  first.scrollIntoView = vi.fn();
  await fireEvent.click(first);
  await fireEvent.click(
    container.querySelector(".lucide-ellipsis")!.closest("button")!,
  );
  await fireEvent.click(await screen.findByText("Move to trash"));
  await waitFor(() =>
    expect(screen.queryByRole("option", { name: /Part 1/ })).toBeNull(),
  );

  const recreated = new PredefinedModelStreamManager(models);
  const fetch = vi.spyOn(recreated, "fetch");
  await rerender({ modelStream: recreated });
  await waitFor(() => expect(fetch).toHaveBeenCalled());
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(screen.queryByRole("option", { name: /Part 2/ })).toBeTruthy();
  expect(screen.queryByRole("option", { name: /Part 1/ })).toBeNull();
});

it("does not reload a deleted model from a predefined stream after searching", async () => {
  const { models } = calibrationParts();
  getContainer().addSingleton(IModelApi, {
    deleteModels: vi.fn().mockResolvedValue(undefined),
  });
  const stream = new PredefinedModelStreamManager(models);
  const { container } = render(ModelGrid, {
    modelStream: stream,
    initialEditMode: true,
  });

  const first = await screen.findByRole("option", { name: /Part 1/ });
  first.scrollIntoView = vi.fn();
  await fireEvent.click(first);
  await fireEvent.click(
    container.querySelector(".lucide-ellipsis")!.closest("button")!,
  );
  await fireEvent.click(await screen.findByText("Move to trash"));
  await waitFor(() =>
    expect(screen.queryByRole("option", { name: /Part 1/ })).toBeNull(),
  );

  await fireEvent.input(screen.getByPlaceholderText("Search"), {
    target: { value: "Part 1" },
  });
  await waitFor(() =>
    expect(screen.queryByRole("option", { name: /Part 2/ })).toBeNull(),
  );
  expect(screen.queryByRole("option", { name: /Part 1/ })).toBeNull();
});

it("keeps a newly selected group when another group's model deletion completes", async () => {
  const { meta, models } = calibrationParts();
  let finishDelete!: () => void;
  const deleteModels = vi.fn(
    () => new Promise<void>((resolve) => (finishDelete = resolve)),
  );
  getContainer().addSingleton(IModelApi, { deleteModels });
  const otherMeta = createGroupMetaInstance(
    8,
    "Other parts",
    "2026-10-24T12:00:00Z",
    "2026-10-24T12:00:00Z",
    "group-8",
  );
  const otherModel = createModelInstance(
    3,
    "Part 3",
    createBlobInstance(3, "hash-3", "stl.zip", 1024, "2026-10-24T12:00:00Z"),
    null,
    null,
    "2026-10-24T12:00:00Z",
    "2026-10-24T12:00:00Z",
    otherMeta,
    [],
    [],
    "model-3",
  );
  const stream = emptyStream();
  stream.fetch.mockResolvedValueOnce([
    createGroupInstance(meta, models, [], null, []),
    createGroupInstance(otherMeta, [otherModel], [], null, []),
  ]);
  const { container } = render(GroupGrid, {
    groupStream: stream as IGroupStreamManager,
  });

  const first = await screen.findByRole("option", {
    name: /Calibration parts/,
  });
  first.scrollIntoView = vi.fn();
  await fireEvent.click(first);
  await fireEvent.click(
    container.querySelector(".lucide-ellipsis")!.closest("button")!,
  );
  await fireEvent.click(
    await screen.findByText("Move selected models to trash"),
  );
  expect(deleteModels).toHaveBeenCalledWith(models);

  const second = screen.getByRole("option", { name: /Other parts/ });
  second.scrollIntoView = vi.fn();
  await fireEvent.click(second);
  finishDelete();
  await waitFor(() =>
    expect(
      screen.queryByRole("option", { name: /Calibration parts/ }),
    ).toBeNull(),
  );
  expect(
    screen
      .getByRole("option", { name: /Other parts/ })
      .getAttribute("aria-selected"),
  ).toBe("true");
});

it("does not restore stale bulk models when an earlier getAll resolves after search", async () => {
  const { models } = calibrationParts();
  let finishInitial!: (result: typeof models) => void;
  const stream = emptyStream();
  stream.fetch.mockResolvedValueOnce(models).mockResolvedValueOnce([]);
  stream.getAll
    .mockImplementationOnce(
      () => new Promise((resolve) => (finishInitial = resolve)),
    )
    .mockResolvedValueOnce([]);
  render(ModelGrid, {
    modelStream: stream as IModelStreamManager,
    default_show_multiselect_all: true,
  });
  await waitFor(() => expect(stream.getAll).toHaveBeenCalledTimes(1));

  await fireEvent.input(screen.getByPlaceholderText("Search"), {
    target: { value: "unmatched" },
  });
  await waitFor(() => expect(stream.getAll).toHaveBeenCalledTimes(2));
  finishInitial(models);
  await new Promise((resolve) => setTimeout(resolve, 0));

  await waitFor(() =>
    expect(screen.queryByText("No models to display")).toBeTruthy(),
  );
  expect(screen.queryByText("2 models")).toBeNull();
});

it("does not report an empty group when an unloaded model remains", async () => {
  const { models } = calibrationParts();
  getContainer().addSingleton(IModelApi, {
    deleteModels: vi.fn().mockResolvedValue(undefined),
  });
  const stream = emptyStream();
  stream.fetch.mockResolvedValueOnce([models[0]]);
  stream.getAll.mockResolvedValue(models);
  const onEmpty = vi.fn();
  const { container } = render(ModelGrid, {
    modelStream: stream as IModelStreamManager,
    initialEditMode: true,
    onEmpty,
  });
  await screen.findByRole("option", { name: /Part 1/ });
  await waitFor(() => expect(stream.getAll).toHaveBeenCalled());
  await fireEvent.click(
    container.querySelector(".lucide-ellipsis")!.closest("button")!,
  );
  await fireEvent.click(await screen.findByText("Move to trash"));
  await waitFor(() =>
    expect(screen.queryByRole("option", { name: /Part 1/ })).toBeNull(),
  );
  expect(onEmpty).not.toHaveBeenCalled();
});

it("can load a new search after a failed model fetch", async () => {
  const { models } = calibrationParts();
  const stream = emptyStream();
  stream.fetch.mockRejectedValueOnce(new Error("temporary failure"));
  stream.fetch.mockResolvedValueOnce([models[1]]);
  render(ModelGrid, { modelStream: stream as IModelStreamManager });

  await waitFor(() => expect(stream.fetch).toHaveBeenCalledTimes(1));
  await fireEvent.input(screen.getByPlaceholderText("Search"), {
    target: { value: "Part 2" },
  });

  expect(await screen.findByRole("option", { name: /Part 2/ })).toBeTruthy();
  expect(stream.fetch).toHaveBeenCalledTimes(2);
});

it("can load a new search after a failed group fetch", async () => {
  const { meta, models } = calibrationParts();
  const group = createGroupInstance(meta, models, [], null, []);
  const stream = emptyStream();
  stream.fetch.mockRejectedValueOnce(new Error("temporary failure"));
  stream.fetch.mockResolvedValueOnce([group]);
  render(GroupGrid, { groupStream: stream as IGroupStreamManager });

  await waitFor(() => expect(stream.fetch).toHaveBeenCalledTimes(1));
  await fireEvent.input(screen.getByPlaceholderText("Search"), {
    target: { value: "Calibration" },
  });

  expect(
    await screen.findByRole("option", { name: /Calibration parts/ }),
  ).toBeTruthy();
  expect(stream.fetch).toHaveBeenCalledTimes(2);
});

describe.each([
  { checkboxes: false, rowChildren: 1 },
  { checkboxes: true, rowChildren: 2 },
])(
  "ModelGrid list rows with multiselect checkboxes $checkboxes",
  ({ checkboxes, rowChildren }) => {
    it("lets the option fill the full-width row", async () => {
      configuration.size_option_models = "List_Medium";
      configuration.show_multiselect_checkboxes = checkboxes;
      const stream = emptyStream();
      stream.fetch.mockResolvedValueOnce(calibrationParts().models);
      render(ModelGrid, { modelStream: stream as IModelStreamManager });

      const option = await screen.findByRole("option", { name: /Part 1/ });
      const row = option.parentElement!;

      expect([...row.classList]).toEqual(
        expect.arrayContaining(["flex", "w-full"]),
      );
      expect([...option.classList]).toEqual(
        expect.arrayContaining(["flex-1", "min-w-0"]),
      );
      expect(row.children).toHaveLength(rowChildren);
    });
  },
);

it("closes the selected group preview on mobile and clears the selection", async () => {
  media.width = 800;
  const stream = groupStreamWithModels();
  render(GroupGrid, { groupStream: stream as IGroupStreamManager });

  const option = await screen.findByRole("option", {
    name: /Calibration parts/,
  });
  option.scrollIntoView = vi.fn();
  expect(
    screen.queryByRole("button", { name: "Close model preview" }),
  ).toBeNull();

  await fireEvent.click(option);

  expect(await screen.findByText("Group 'Calibration parts'")).toBeTruthy();
  expect(screen.getByText("2 models")).toBeTruthy();
  expect(
    screen.queryByRole("option", { name: /Calibration parts/ }),
  ).toBeNull();
  await fireEvent.click(
    screen.getByRole("button", { name: "Close model preview" }),
  );

  const restored = await screen.findByRole("option", {
    name: /Calibration parts/,
  });
  expect(restored.getAttribute("aria-selected")).toBe("false");
  expect(screen.queryByText("Group 'Calibration parts'")).toBeNull();
  expect(screen.queryByText("2 models")).toBeNull();
  expect(
    screen.queryByRole("button", { name: "Close model preview" }),
  ).toBeNull();
});

it("uses the normal group view at medium widths even when split mode is configured", async () => {
  media.width = 1400;
  configuration.group_split_view = "split-left-right";
  const stream = groupStreamWithModels();
  render(GroupGrid, { groupStream: stream as IGroupStreamManager });

  const option = await screen.findByRole("option", {
    name: /Calibration parts/,
  });
  option.scrollIntoView = vi.fn();

  await fireEvent.click(option);

  expect(
    screen
      .getByRole("option", { name: /Calibration parts/ })
      .getAttribute("aria-selected"),
  ).toBe("true");
  expect(
    screen.queryByRole("button", { name: "Close model preview" }),
  ).toBeNull();
  expect(
    (await screen.findByRole("link", { name: "View models" })).getAttribute(
      "href",
    ),
  ).toBe("/group/7");
});
