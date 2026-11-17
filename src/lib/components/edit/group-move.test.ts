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
} from "#lib/api/shared/group_api";
import { createLabelMetaInstance, ILabelApi } from "#lib/api/shared/label_api";
import { createModelInstance, IModelApi } from "#lib/api/shared/model_api";
import { ISlicerApi } from "#lib/api/shared/slicer_api";
import { configurationMeta } from "#lib/configuration.svelte";
import ModelEdit from "./model.svelte";
import MultiModelEdit from "./multi-model.svelte";

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
vi.mock("#lib/components/view/three-d-canvas.svelte", () => ({
  default: () => {},
}));

const timestamp = "2026-10-29T12:00:00Z";
const destination = createGroupMetaInstance(
  42,
  "Destination",
  timestamp,
  timestamp,
  "group-42",
);
function model(id: number) {
  return createModelInstance(
    id,
    `Part ${id}`,
    createBlobInstance(id, `hash-${id}`, "part.stl", 1024, timestamp),
    null,
    null,
    timestamp,
    timestamp,
    null,
    [],
    [],
    `model-${id}`,
  );
}
function provideApis(move = vi.fn().mockResolvedValue(undefined)) {
  const container = getContainer();
  const groups = [createGroupInstance(destination, [], [], null, [])];
  container.addSingleton(IGroupApi, {
    getGroups: vi.fn().mockResolvedValue(groups),
    addModelsToGroup: move,
  });
  container.addSingleton(IModelApi, {});
  container.addSingleton(ILabelApi, {});
  container.addSingleton(IBlobApi, {
    getBlobThumbnailUrl: vi.fn().mockResolvedValue("data:image/png;base64,"),
  });
  container.addSingleton(ISlicerApi, {
    availableSlicers: vi.fn().mockResolvedValue([]),
  });
  return move;
}
afterEach(() => {
  cleanup();
  resetContainer();
  configurationMeta.applicationReadOnly = false;
  vi.restoreAllMocks();
});

it("moves a single model to an existing unrelated group", async () => {
  const move = provideApis();
  const part = model(1);
  const label = createLabelMetaInstance(1, "Printed", 0, timestamp, "label-1");
  part.labels = [label];
  render(ModelEdit, { model: part });
  await fireEvent.click(screen.getByRole("button", { name: /move to group/i }));
  await fireEvent.click(await screen.findByText("Destination"));
  await waitFor(() =>
    expect(move).toHaveBeenCalledWith(destination, [
      expect.objectContaining({ id: 1 }),
    ]),
  );
  await waitFor(() => expect(part.group?.id).toBe(42));
  expect(part.labels).toEqual([label]);
});

it("moves all selected models to an existing unrelated group", async () => {
  const move = provideApis();
  const parts = [model(1), model(2)];
  render(MultiModelEdit, { models: parts });
  await fireEvent.click(screen.getByRole("button", { name: /move to group/i }));
  await fireEvent.click(await screen.findByText("Destination"));
  await waitFor(() =>
    expect(move).toHaveBeenCalledWith(destination, [
      expect.objectContaining({ id: 1 }),
      expect.objectContaining({ id: 2 }),
    ]),
  );
  await waitFor(() =>
    expect(parts.map((part) => part.group?.id)).toEqual([42, 42]),
  );
});

it("keeps the existing group when a move fails", async () => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  const move = provideApis(vi.fn().mockRejectedValue(new Error("unavailable")));
  const part = model(1);
  const original = createGroupMetaInstance(
    8,
    "Original",
    timestamp,
    timestamp,
    "group-8",
  );
  part.group = original;
  render(ModelEdit, { model: part });
  await fireEvent.click(screen.getByRole("button", { name: /move to group/i }));
  await fireEvent.click(await screen.findByText("Destination"));
  await waitFor(() => expect(move).toHaveBeenCalled());
  expect(part.group?.id).toBe(8);
});

it("does not offer group changes in read-only mode", () => {
  provideApis();
  configurationMeta.applicationReadOnly = true;
  render(ModelEdit, { model: model(1) });
  expect(screen.queryByRole("button", { name: /move to group/i })).toBeNull();
});

it("does not offer bulk group changes in read-only mode", () => {
  provideApis();
  configurationMeta.applicationReadOnly = true;
  render(MultiModelEdit, { models: [model(1), model(2)] });
  expect(screen.queryByRole("button", { name: /move to group/i })).toBeNull();
});

it("disables the current group as a destination", async () => {
  const move = provideApis();
  const part = model(1);
  part.group = destination;
  render(ModelEdit, { model: part });
  await fireEvent.click(screen.getByRole("button", { name: /move to group/i }));
  const target = await screen.findByRole("button", { name: "Destination" });
  expect((target as HTMLButtonElement).disabled).toBe(true);
  expect(move).not.toHaveBeenCalled();
});

it("shows a retry when loading destinations fails", async () => {
  const container = getContainer();
  provideApis();
  container.addSingleton(IGroupApi, {
    getGroups: vi
      .fn()
      .mockRejectedValueOnce(new Error("unavailable"))
      .mockResolvedValue([]),
  });
  vi.spyOn(console, "error").mockImplementation(() => {});
  render(ModelEdit, { model: model(1) });
  await fireEvent.click(screen.getByRole("button", { name: /move to group/i }));
  await waitFor(() => expect(screen.queryByRole("alert")).not.toBeNull());
  expect(screen.getByRole("alert").textContent).toContain(
    "Could not load groups",
  );
  await fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(await screen.findByText("No groups found")).toBeTruthy();
});
