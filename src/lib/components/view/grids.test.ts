// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { getContainer, resetContainer } from "#lib/api/dependency_injection";
import { IBlobApi, createBlobInstance } from "#lib/api/shared/blob_api";
import {
  IModelApi,
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

afterEach(() => {
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

function groupStreamWithModels() {
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
  const group = createGroupInstance(meta, models, [], null, []);
  const stream = emptyStream();
  stream.fetch.mockResolvedValueOnce([group]);

  return stream;
}

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
