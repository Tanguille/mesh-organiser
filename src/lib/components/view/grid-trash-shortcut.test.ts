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
import { configuration, configurationMeta } from "#lib/configuration.svelte";
import { configurationDefault } from "#lib/api/shared/settings_api";
import ModelGrid from "./model-grid.svelte";

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
  Object.assign(configuration, configurationDefault());
  configurationMeta.applicationReadOnly = false;
});

async function setup(deleteModels = vi.fn().mockResolvedValue(undefined)) {
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
    fetch: vi.fn().mockResolvedValueOnce(models).mockResolvedValue([]),
    getAll: vi.fn().mockResolvedValue(models),
  };
  render(ModelGrid, { modelStream: stream as IModelStreamManager });
  await screen.findByRole("option", { name: /Part 4/ });
  return {
    models,
    option: (id: number) =>
      screen.getByRole("option", { name: new RegExp(`Part ${id}`) }),
  };
}

it("moves only the selected model to trash on Delete", async () => {
  const deleteModels = vi.fn().mockResolvedValue(undefined);
  const { models, option } = await setup(deleteModels);
  await fireEvent.click(option(1));
  await fireEvent.keyDown(option(1), { key: "Delete" });
  await waitFor(() =>
    expect(deleteModels).toHaveBeenCalledExactlyOnceWith([models[0]]),
  );
  await waitFor(() =>
    expect(screen.queryByRole("option", { name: /Part 1/ })).toBeNull(),
  );
  expect(option(2)).toBeTruthy();
});

it("keeps a keyboard tab stop and returns focus to a remaining model after deletion", async () => {
  const { option } = await setup();
  await fireEvent.click(option(2));
  option(2).focus();
  await fireEvent.keyDown(option(2), { key: "Delete" });
  await waitFor(() =>
    expect(screen.queryByRole("option", { name: /Part 2/ })).toBeNull(),
  );
  const remaining = screen
    .getAllByRole("option")
    .filter((element) => element.getAttribute("tabindex") === "0");
  expect(remaining).toHaveLength(1);
  expect(document.activeElement).toBe(remaining[0]);
});

it("moves exactly the multiple selected models to trash", async () => {
  const deleteModels = vi.fn().mockResolvedValue(undefined);
  const { models, option } = await setup(deleteModels);
  await fireEvent.click(option(1));
  await fireEvent.click(option(3), { ctrlKey: true });
  await fireEvent.keyDown(option(3), { key: "Delete" });
  await waitFor(() =>
    expect(deleteModels).toHaveBeenCalledExactlyOnceWith([
      models[0],
      models[2],
    ]),
  );
  expect(option(2)).toBeTruthy();
});

it("ignores repeated Delete while an operation is pending", async () => {
  let finish!: () => void;
  const deleteModels = vi
    .fn()
    .mockImplementation(
      () => new Promise<void>((resolve) => (finish = resolve)),
    );
  const { option } = await setup(deleteModels);
  await fireEvent.click(option(1));
  await fireEvent.keyDown(option(1), { key: "Delete" });
  await fireEvent.keyDown(option(1), { key: "Delete", repeat: true });
  await fireEvent.keyDown(option(1), { key: "Delete" });
  await waitFor(() => expect(deleteModels).toHaveBeenCalledTimes(1));
  finish();
});

it("does not move models to trash when Delete is pressed in a search input", async () => {
  const deleteModels = vi.fn().mockResolvedValue(undefined);
  const { option } = await setup(deleteModels);
  await fireEvent.click(option(1));
  const input = screen.getByPlaceholderText("Search");
  input.focus();
  const event = new KeyboardEvent("keydown", {
    key: "Delete",
    bubbles: true,
    cancelable: true,
  });
  await fireEvent(input, event);
  expect(event.defaultPrevented).toBe(false);
  expect(deleteModels).not.toHaveBeenCalled();
  expect(option(1)).toBeTruthy();
});

it("does not delete models in a read-only view", async () => {
  const deleteModels = vi.fn().mockResolvedValue(undefined);
  configurationMeta.applicationReadOnly = true;
  const { option } = await setup(deleteModels);
  await fireEvent.click(option(1));
  await fireEvent.keyDown(option(1), { key: "Delete" });
  expect(deleteModels).not.toHaveBeenCalled();
  expect(option(1)).toBeTruthy();
});

it("keeps the selected model visible after a failed trash request", async () => {
  const deleteModels = vi.fn().mockRejectedValue(new Error("offline"));
  const { option } = await setup(deleteModels);
  await fireEvent.click(option(1));
  await fireEvent.keyDown(option(1), { key: "Delete" });
  await waitFor(() => expect(deleteModels).toHaveBeenCalledTimes(1));
  expect(option(1).getAttribute("aria-selected")).toBe("true");
});
