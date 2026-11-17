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
const notify = vi.hoisted(() => ({
  success: vi.fn(),
  error: vi.fn(),
  promise: vi.fn(),
  goto: vi.fn(),
}));
vi.mock("$app/navigation", () => ({ goto: notify.goto }));
vi.mock("svelte-sonner", () => ({
  toast: {
    success: notify.success,
    error: notify.error,
    promise: notify.promise,
  },
}));
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

it("confirms a single model was moved and offers a View trash recovery action", async () => {
  provideApis();
  const deleteModels = vi.fn().mockResolvedValue(undefined);
  getContainer().addSingleton(IModelApi, { deleteModels });
  render(ModelEdit, { model: model(1) });
  await fireEvent.click(screen.getByRole("button", { name: /move to trash/i }));
  await waitFor(() => expect(deleteModels).toHaveBeenCalled());
  await waitFor(() => expect(notify.success).toHaveBeenCalled());
  const [message, options] = notify.success.mock.calls.at(-1)!;
  expect(message).toMatch(/moved.*trash/i);
  expect(options.action.label).toMatch(/view trash/i);
  options.action.onClick();
  expect(notify.goto).toHaveBeenCalledWith("/trash");
});
