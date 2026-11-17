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
import {
  IGroupApi,
  createGroupInstance,
  createGroupMetaInstance,
} from "#lib/api/shared/group_api";
import { ILabelApi, createLabelMetaInstance } from "#lib/api/shared/label_api";
import {
  IModelApi,
  createModelInstance,
  type IModelStreamManager,
} from "#lib/api/shared/model_api";
import { IResourceApi } from "#lib/api/shared/resource_api";
import { ISlicerApi } from "#lib/api/shared/slicer_api";
import { sidebarState } from "#lib/sidebar_data.svelte";
import { stopDragging } from "#lib/drag-selected-models.svelte";
import GridFixture from "./test/model-grid-drop-fixture.svelte";

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
  stopDragging();
  resetContainer();
  sidebarState.labels = [];
  vi.restoreAllMocks();
});

async function setup(
  groupAction = vi.fn().mockResolvedValue(undefined),
  labelAction = vi.fn().mockResolvedValue(undefined),
) {
  const date = "2026-10-24T12:00:00Z";
  const meta = createGroupMetaInstance(
    7,
    "Destination group",
    date,
    date,
    "group-7",
  );
  const group = createGroupInstance(meta, [], [], null, []);
  const label = createLabelMetaInstance(9, "Blue label", 0, date, "label-9");
  const models = [1, 2].map((id) =>
    createModelInstance(
      id,
      `Part ${id}`,
      createBlobInstance(id, `hash-${id}`, "stl.zip", 100, date),
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
  const container = getContainer();
  container.addSingleton(IBlobApi, {
    getBlobThumbnailUrl: vi.fn().mockResolvedValue("data:image/png;base64,"),
  });
  container.addSingleton(IGroupApi, {
    getGroups: vi.fn().mockResolvedValue([group]),
    addModelsToGroup: groupAction,
  });
  container.addSingleton(ILabelApi, { addLabelToModels: labelAction });
  container.addSingleton(IModelApi, {});
  container.addSingleton(IResourceApi, {
    getResources: vi.fn().mockResolvedValue([]),
  });
  container.addSingleton(ISlicerApi, {
    availableSlicers: vi.fn().mockResolvedValue([]),
  });
  sidebarState.labels = [
    {
      meta: label,
      children: [],
      effectiveLabels: [],
      hasParent: false,
      modelCount: 0,
      groupCount: 0,
      selfModelCount: 0,
      selfGroupCount: 0,
    },
  ];
  const stream = {
    setSearchText: vi.fn(),
    setOrderBy: vi.fn(),
    setFileTypes: vi.fn(),
    fetch: vi.fn().mockResolvedValueOnce(models).mockResolvedValue([]),
    getAll: vi.fn().mockResolvedValue(models),
  };
  render(GridFixture, { modelStream: stream as IModelStreamManager });
  const first = await screen.findByRole("option", { name: /Part 1/ });
  await fireEvent.click(first);
  return { first, groupAction, labelAction };
}

async function dragTo(first: HTMLElement, target: string) {
  await fireEvent.mouseDown(first, { button: 0, clientX: 0, clientY: 0 });
  await fireEvent.mouseMove(first, { clientX: 20 });
  await fireEvent.mouseUp(screen.getByTestId(target));
}

it("updates the selected model's group after a successful group drop", async () => {
  const { first, groupAction } = await setup();
  await dragTo(first, "group-target");
  await waitFor(() => expect(groupAction).toHaveBeenCalledTimes(1));
  await waitFor(() =>
    expect(
      screen.queryByRole("link", { name: "Destination group" }),
    ).not.toBeNull(),
  );
});

it("updates the selected model's labels after a successful label drop", async () => {
  const { first, labelAction } = await setup();
  await dragTo(first, "label-target");
  await waitFor(() => expect(labelAction).toHaveBeenCalledTimes(1));
  await waitFor(() => expect(screen.queryByText("Blue label")).not.toBeNull());
});

it("leaves the selected model unchanged when a group move fails", async () => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  const groupAction = vi.fn().mockRejectedValue(new Error("offline"));
  const { first } = await setup(groupAction);
  await dragTo(first, "group-target");
  await waitFor(() => expect(groupAction).toHaveBeenCalledTimes(1));
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(screen.queryByRole("link", { name: "Destination group" })).toBeNull();
});

it("leaves the selected model unchanged when a label drop fails", async () => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  const labelAction = vi.fn().mockRejectedValue(new Error("offline"));
  const { first } = await setup(vi.fn(), labelAction);
  await dragTo(first, "label-target");
  await waitFor(() => expect(labelAction).toHaveBeenCalledTimes(1));
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(screen.queryByText("Blue label")).toBeNull();
});
