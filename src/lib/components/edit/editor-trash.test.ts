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
import { ILabelApi } from "#lib/api/shared/label_api";
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

it("moves a single model to trash without edit mode", async () => {
  provideApis();
  const deleteModels = vi.fn().mockResolvedValue(undefined);
  getContainer().addSingleton(IModelApi, { deleteModels });
  const onDelete = vi.fn();
  render(ModelEdit, { model: model(1), onDelete });
  await fireEvent.click(screen.getByRole("button", { name: /move to trash/i }));
  await waitFor(() =>
    expect(deleteModels).toHaveBeenCalledWith([
      expect.objectContaining({ id: 1 }),
    ]),
  );
  await waitFor(() => expect(onDelete).toHaveBeenCalled());
});

it("moves all selected models to trash", async () => {
  provideApis();
  const deleteModels = vi.fn().mockResolvedValue(undefined);
  getContainer().addSingleton(IModelApi, { deleteModels });
  render(MultiModelEdit, { models: [model(1), model(2)] });
  await fireEvent.click(screen.getByRole("button", { name: /move to trash/i }));
  await waitFor(() =>
    expect(deleteModels).toHaveBeenCalledWith([
      expect.objectContaining({ id: 1 }),
      expect.objectContaining({ id: 2 }),
    ]),
  );
});

it("prevents repeated trash requests while the first is pending", async () => {
  provideApis();
  let finish!: () => void;
  const deleteModels = vi.fn().mockImplementation(
    () =>
      new Promise<void>((resolve) => {
        finish = resolve;
      }),
  );
  getContainer().addSingleton(IModelApi, { deleteModels });
  render(ModelEdit, { model: model(1) });
  const button = screen.getByRole("button", { name: /move to trash/i });
  await fireEvent.click(button);
  expect((button as HTMLButtonElement).disabled).toBe(true);
  await fireEvent.click(button);
  expect(deleteModels).toHaveBeenCalledTimes(1);
  finish();
});

it.each(["single", "multiple"])(
  "does not offer trash in read-only %s editor",
  (kind) => {
    provideApis();
    configurationMeta.applicationReadOnly = true;
    if (kind === "single") render(ModelEdit, { model: model(1) });
    else render(MultiModelEdit, { models: [model(1), model(2)] });
    expect(screen.queryByRole("button", { name: /move to trash/i })).toBeNull();
  },
);
