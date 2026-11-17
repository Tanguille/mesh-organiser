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
import { IGroupApi } from "#lib/api/shared/group_api";
import { ILabelApi } from "#lib/api/shared/label_api";
import {
  IModelApi,
  createModelInstance,
  type IModelStreamManager,
} from "#lib/api/shared/model_api";
import { IResourceApi } from "#lib/api/shared/resource_api";
import { ISlicerApi } from "#lib/api/shared/slicer_api";
import { configuration } from "#lib/configuration.svelte";
import { configurationDefault } from "#lib/api/shared/settings_api";
import ModelGrid from "./model-grid.svelte";
import ModelGridDragFixture from "./test/model-grid-drag-fixture.svelte";
import {
  startDragging,
  state as dragState,
  stopDragging,
} from "#lib/drag-selected-models.svelte";

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

afterEach(() => {
  cleanup();
  resetContainer();
  stopDragging();
  Object.assign(configuration, configurationDefault());
});

async function setup(withDragRoot = false, filteredResults = false) {
  const container = getContainer();
  container.addSingleton(IBlobApi, {
    getBlobThumbnailUrl: vi.fn().mockResolvedValue("data:image/png;base64,"),
  });
  container.addSingleton(IGroupApi, {});
  container.addSingleton(IModelApi, { deleteModels: vi.fn() });
  container.addSingleton(ILabelApi, {});
  container.addSingleton(IResourceApi, {
    getResources: vi.fn().mockResolvedValue([]),
  });
  container.addSingleton(ISlicerApi, {
    availableSlicers: vi.fn().mockResolvedValue([]),
  });
  const date = "2026-10-24T12:00:00Z";
  const models = [1, 2, 3, 4].map((id) =>
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
  const stream = {
    setSearchText: vi.fn(),
    setOrderBy: vi.fn(),
    setFileTypes: vi.fn(),
    fetch: vi
      .fn()
      .mockResolvedValueOnce(models)
      .mockResolvedValueOnce(filteredResults ? models.slice(2) : [])
      .mockResolvedValue([]),
    getAll: vi.fn().mockResolvedValue(models),
  };
  render(withDragRoot ? ModelGridDragFixture : ModelGrid, {
    modelStream: stream as IModelStreamManager,
  });
  await screen.findByRole("option", { name: /Part 4/ });
  return (id: number) =>
    screen.getByRole("option", { name: new RegExp(`Part ${id}`) });
}

it.each(["ctrlKey", "metaKey"] as const)(
  "selects all loaded models with %s+A",
  async (modifier) => {
    const option = await setup();
    option(1).focus();
    await fireEvent.keyDown(option(1), { key: "a", [modifier]: true });
    for (const id of [1, 2, 3, 4])
      expect(option(id).getAttribute("aria-selected")).toBe("true");
  },
);

it("clears the selected models with Escape", async () => {
  const option = await setup();
  await fireEvent.click(option(1));
  await fireEvent.keyDown(option(1), { key: "Escape" });
  expect(option(1).getAttribute("aria-selected")).toBe("false");
});

it("starts a new range when the old anchor disappears after filtering", async () => {
  const option = await setup(false, true);
  await fireEvent.click(option(1));
  await fireEvent.input(screen.getByPlaceholderText("Search"), {
    target: { value: "part" },
  });
  await waitFor(() =>
    expect(screen.queryByRole("option", { name: /Part 1/ })).toBeNull(),
  );
  await fireEvent.click(option(4), { shiftKey: true });
  expect(option(4).getAttribute("aria-selected")).toBe("true");
});

it("grows and shrinks a range from its original anchor with repeated shift clicks", async () => {
  const option = await setup();
  await fireEvent.click(option(1));
  await fireEvent.click(option(3), { shiftKey: true });
  await fireEvent.click(option(4), { shiftKey: true });
  await waitFor(() =>
    expect(option(2).getAttribute("aria-selected")).toBe("true"),
  );
  expect(option(1).getAttribute("aria-selected")).toBe("true");
  await fireEvent.click(option(2), { shiftKey: true });
  expect(option(1).getAttribute("aria-selected")).toBe("true");
  expect(option(2).getAttribute("aria-selected")).toBe("true");
  expect(option(3).getAttribute("aria-selected")).toBe("false");
  expect(option(4).getAttribute("aria-selected")).toBe("false");
});

it("moves keyboard focus to the next model with ArrowRight", async () => {
  const option = await setup();
  option(1).focus();
  await fireEvent.keyDown(option(1), { key: "ArrowRight" });
  expect(document.activeElement).toBe(option(2));
});

it("extends a selection from a focused model on the first Shift+Arrow", async () => {
  const option = await setup();
  option(1).focus();
  await fireEvent.keyDown(option(1), { key: "ArrowRight", shiftKey: true });
  expect(option(1).getAttribute("aria-selected")).toBe("true");
  expect(option(2).getAttribute("aria-selected")).toBe("true");
});

it("moves focus to the first and last model with Home and End", async () => {
  const option = await setup();
  option(2).focus();
  await fireEvent.keyDown(option(2), { key: "End" });
  expect(document.activeElement).toBe(option(4));
  await fireEvent.keyDown(option(4), { key: "Home" });
  expect(document.activeElement).toBe(option(1));
});

it("exposes one tab stop for the model collection", async () => {
  await setup();
  const listbox = screen.queryByRole("listbox", { name: /models/i });
  expect(listbox).not.toBeNull();
  expect(listbox!.getAttribute("aria-multiselectable")).toBe("true");
  expect(
    listbox!.querySelectorAll('[role="option"][tabindex="0"]'),
  ).toHaveLength(1);
});

it("does not apply selection shortcuts when the search input has focus", async () => {
  const option = await setup();
  await fireEvent.click(option(1));
  const search = screen.getByPlaceholderText("Search");
  search.focus();
  const event = new KeyboardEvent("keydown", {
    key: "a",
    ctrlKey: true,
    bubbles: true,
    cancelable: true,
  });
  await fireEvent(search, event);
  expect(event.defaultPrevented).toBe(false);
  expect(option(1).getAttribute("aria-selected")).toBe("true");
  expect(option(2).getAttribute("aria-selected")).toBe("false");
});

it("cancels dragging with Escape before clearing the focused selection", async () => {
  const option = await setup(true);
  await fireEvent.click(option(1));
  option(1).focus();
  startDragging([]);
  await fireEvent.keyDown(option(1), { key: "Escape" });
  expect(dragState.dragging).toBe(false);
  expect(option(1).getAttribute("aria-selected")).toBe("true");
});

it.each(["Space", "Shift+ArrowRight"])(
  "applies %s after dragging a previously unselected model",
  async (shortcut) => {
    const option = await setup(true);
    const first = option(1);
    await fireEvent.mouseDown(first, { button: 0, clientX: 0, clientY: 0 });
    await fireEvent.mouseMove(first, { clientX: 20, clientY: 0 });
    expect(dragState.dragging).toBe(true);
    await fireEvent.mouseUp(first);
    await fireEvent.click(first);
    expect(first.getAttribute("aria-selected")).toBe("true");

    first.focus();
    if (shortcut === "Space") {
      await fireEvent.keyDown(first, { key: " " });
      expect(first.getAttribute("aria-selected")).toBe("false");
    } else {
      await fireEvent.keyDown(first, { key: "ArrowRight", shiftKey: true });
      expect(option(2).getAttribute("aria-selected")).toBe("true");
    }
  },
);
