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

it("does not expose model editing in readonly even if initial edit mode was requested", async () => {
  provideApis();
  configurationMeta.applicationReadOnly = true;
  const { container } = render(ModelEdit, {
    model: model(1),
    initialEditMode: true,
  });
  expect(screen.queryByLabelText("Name")).toBeNull();
  expect(container.querySelector(".lucide-ellipsis")).toBeNull();
  expect(screen.queryByRole("button", { name: /move to trash/i })).toBeNull();
});

it("reports the original membership after ungrouping a single model", async () => {
  provideApis();
  const removeModelsFromGroup = vi.fn().mockResolvedValue(undefined);
  getContainer().addSingleton(IGroupApi, { removeModelsFromGroup });
  const part = model(1);
  part.group = createGroupMetaInstance(
    8,
    "Original",
    timestamp,
    timestamp,
    "group-8",
  );
  const onGroupDelete = vi.fn();
  const { container } = render(ModelEdit, {
    model: part,
    initialEditMode: true,
    onGroupDelete,
  } as { model: ReturnType<typeof model>; initialEditMode: boolean });
  await fireEvent.click(
    container.querySelector(".lucide-ellipsis")!.closest("button")!,
  );
  await fireEvent.click(await screen.findByText("Remove from current group"));
  await waitFor(() =>
    expect(removeModelsFromGroup).toHaveBeenCalledWith([
      expect.objectContaining({ id: 1 }),
    ]),
  );
  await waitFor(() =>
    expect(onGroupDelete).toHaveBeenCalledWith([
      expect.objectContaining({
        id: 1,
        group: expect.objectContaining({ id: 8 }),
      }),
    ]),
  );
  expect(part.group).toBeNull();
});
