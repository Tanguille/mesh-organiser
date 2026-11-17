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
import GroupEdit from "./group.svelte";
import { IResourceApi } from "#lib/api/shared/resource_api";

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
    createBlobInstance(id, `hash-${id}`, "stl", 1024, timestamp),
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

it("names the model edit and properties controls", () => {
  provideApis();
  render(ModelEdit, { model: model(1) });
  expect(screen.queryByRole("button", { name: "Edit model" })).not.toBeNull();
  expect(
    screen.queryByRole("button", { name: "Model properties" }),
  ).not.toBeNull();
});

it("names the 3D preview and rotation controls", async () => {
  provideApis();
  render(ModelEdit, { model: model(1) });
  const preview = screen.queryByRole("button", { name: "Toggle 3D preview" });
  expect(preview).not.toBeNull();
  if (preview!.getAttribute("aria-pressed") === "false")
    await fireEvent.click(preview!);
  await waitFor(() =>
    expect(
      screen.queryByRole("button", { name: "Pause model rotation" }),
    ).not.toBeNull(),
  );
});

it("names the group edit control", async () => {
  provideApis();
  getContainer().addSingleton(IResourceApi, {
    getResources: vi.fn().mockResolvedValue([]),
  });
  const group = createGroupInstance(destination, [model(1)], [], null, []);
  render(GroupEdit, { group, settingsVertical: true });
  expect(screen.queryByRole("button", { name: "Edit group" })).not.toBeNull();
});
