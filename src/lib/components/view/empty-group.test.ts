// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, expect, it, vi } from "vitest";
import { getContainer, resetContainer } from "#lib/api/dependency_injection";
import { createBlobInstance, IBlobApi } from "#lib/api/shared/blob_api";
import {
  createGroupInstance,
  createGroupMetaInstance,
  IGroupApi,
  type IGroupStreamManager,
} from "#lib/api/shared/group_api";
import { ILabelApi } from "#lib/api/shared/label_api";
import { createModelInstance, IModelApi } from "#lib/api/shared/model_api";
import { IResourceApi } from "#lib/api/shared/resource_api";
import { IResourceFolderApi } from "#lib/api/shared/resource_folder_api";
import { ISlicerApi } from "#lib/api/shared/slicer_api";
import { configuration, configurationMeta } from "#lib/configuration.svelte";
import { configurationDefault } from "#lib/api/shared/settings_api";
import GroupPage from "./group-page.svelte";
import GroupGrid from "./group-grid.svelte";

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
function setup() {
  const container = getContainer();
  container.addSingleton(IBlobApi, {
    getBlobThumbnailUrl: vi.fn().mockResolvedValue("data:image/png;base64,"),
  });
  container.addSingleton(IGroupApi, {
    getGroups: vi.fn().mockResolvedValue([]),
  });
  container.addSingleton(IModelApi, {});
  container.addSingleton(ILabelApi, {});
  container.addSingleton(IResourceApi, {
    getResources: vi.fn().mockResolvedValue([]),
  });
  container.addSingleton(ISlicerApi, {
    availableSlicers: vi.fn().mockResolvedValue([]),
  });
  const date = "2026-10-24T12:00:00Z";
  const models = [1, 2].map((id) =>
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
  const group = createGroupInstance(
    createGroupMetaInstance(7, "Calibration", date, date, "group-7"),
    models,
    [],
    null,
    [],
  );
  const stream = (items: unknown[]) => ({
    setSearchText: vi.fn(),
    setOrderBy: vi.fn(),
    setFileTypes: vi.fn(),
    fetch: vi.fn().mockResolvedValueOnce(items).mockResolvedValue([]),
    getAll: vi.fn().mockResolvedValue(items),
  });
  return { models, group, stream };
}

it("renders and selects an empty group as a drop destination", async () => {
  const { group, stream } = setup();
  group.models = [];
  render(GroupGrid, { groupStream: stream([group]) as IGroupStreamManager });
  const option = await screen.findByRole("option", { name: /Calibration/ });
  expect(option.getAttribute("data-drag-type")).toBe("group");
  expect(option.getAttribute("data-drag-param")).toBe(String(group.meta.id));
  await fireEvent.click(option);
  expect(await screen.findByText("Group 'Calibration'")).toBeTruthy();
});

it("opens an empty group page and retains its group drop target", async () => {
  const { group } = setup();
  group.models = [];
  getContainer().addSingleton(IGroupApi, {
    getGroups: vi.fn().mockResolvedValue([group]),
  });
  const view = render(GroupPage, { group: { id: group.meta.id } });
  expect(await screen.findByText("Group 'Calibration'")).toBeTruthy();
  expect(
    view.container.querySelector(
      `[data-drag-type="group"][data-drag-param="${group.meta.id}"]`,
    ),
  ).not.toBeNull();
});

it("does not offer unsupported project assignment or creation for an empty group", async () => {
  const { group } = setup();
  group.models = [];
  getContainer().addSingleton(IGroupApi, {
    getGroups: vi.fn().mockResolvedValue([group]),
  });
  getContainer().addSingleton(IResourceFolderApi, {
    openResourceFolder: vi.fn(),
  });
  render(GroupPage, { group: { id: group.meta.id } });
  await screen.findByText("Group 'Calibration'");
  expect(screen.queryByText("Project")).toBeNull();
  expect(screen.queryByRole("button", { name: /create project/i })).toBeNull();
});
