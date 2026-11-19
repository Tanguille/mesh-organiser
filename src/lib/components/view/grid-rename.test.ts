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
  type IGroupStreamManager,
} from "#lib/api/shared/group_api";
import { ILabelApi } from "#lib/api/shared/label_api";
import {
  createModelInstance,
  IModelApi,
  type IModelStreamManager,
} from "#lib/api/shared/model_api";
import { IResourceApi } from "#lib/api/shared/resource_api";
import { ISlicerApi } from "#lib/api/shared/slicer_api";
import { configuration, configurationMeta } from "#lib/configuration.svelte";
import { configurationDefault } from "#lib/api/shared/settings_api";
import ModelGrid from "./model-grid.svelte";
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

it("F2 edits the selected model name and selects its text", async () => {
  const { models, stream } = setup();
  render(ModelGrid, { modelStream: stream(models) as IModelStreamManager });
  const option = await screen.findByRole("option", { name: /Part 1/ });
  await fireEvent.click(option);
  option.focus();
  await fireEvent.keyDown(option, { key: "F2" });
  const input = (await screen.findByLabelText("Name")) as HTMLInputElement;
  await waitFor(() => expect(document.activeElement).toBe(input));
  expect(input.value).toBe("Part 1");
  expect([input.selectionStart, input.selectionEnd]).toEqual([
    0,
    input.value.length,
  ]);
});

it("F2 edits the selected group name and selects its text", async () => {
  const { group, stream } = setup();
  render(GroupGrid, { groupStream: stream([group]) as IGroupStreamManager });
  const option = await screen.findByRole("option", { name: /Calibration/ });
  await fireEvent.click(option);
  option.focus();
  await fireEvent.keyDown(option, { key: "F2" });
  const input = (await screen.findByLabelText("Name")) as HTMLInputElement;
  await waitFor(() => expect(document.activeElement).toBe(input));
  expect(input.value).toBe("Calibration");
  expect([input.selectionStart, input.selectionEnd]).toEqual([
    0,
    input.value.length,
  ]);
});

it("does not enable model renaming with F2 in read-only mode", async () => {
  const { models, stream } = setup();
  configurationMeta.applicationReadOnly = true;
  render(ModelGrid, { modelStream: stream(models) as IModelStreamManager });
  const option = await screen.findByRole("option", { name: /Part 1/ });
  await fireEvent.click(option);
  await fireEvent.keyDown(option, { key: "F2" });
  expect(screen.queryByLabelText("Name")).toBeNull();
});

it("F2 renames the group rather than its only model", async () => {
  const { group, stream } = setup();
  group.models = group.models.slice(0, 1);
  render(GroupGrid, { groupStream: stream([group]) as IGroupStreamManager });
  const option = await screen.findByRole("option", { name: /Calibration/ });
  await fireEvent.click(option);
  option.focus();
  await fireEvent.keyDown(option, { key: "F2" });
  await waitFor(() =>
    expect((document.activeElement as HTMLInputElement).value).toBe(
      "Calibration",
    ),
  );
  expect(screen.getAllByLabelText("Name")).toHaveLength(1);
});

it("does not reopen model rename after selection is cleared and another model selected", async () => {
  const { models, stream } = setup();
  render(ModelGrid, { modelStream: stream(models) as IModelStreamManager });
  const first = await screen.findByRole("option", { name: /Part 1/ });
  const second = await screen.findByRole("option", { name: /Part 2/ });
  await fireEvent.click(first);
  first.focus();
  await fireEvent.keyDown(first, { key: "F2" });
  const name = (await screen.findByLabelText("Name")) as HTMLInputElement;
  await waitFor(() => expect(document.activeElement).toBe(name));
  first.focus();
  await fireEvent.keyDown(first, { key: "Escape" });
  await fireEvent.click(second);
  second.focus();
  await waitFor(() => expect(document.activeElement).toBe(second));
  expect(screen.queryByLabelText("Name")).toBeNull();
});

it("does not reopen group rename after selection is cleared and another group selected", async () => {
  const { group, stream } = setup();
  const date = "2026-10-24T12:00:00Z";
  const secondGroup = createGroupInstance(
    createGroupMetaInstance(8, "Fasteners", date, date, "group-8"),
    [],
    [],
    null,
    [],
  );
  render(GroupGrid, {
    groupStream: stream([group, secondGroup]) as IGroupStreamManager,
  });
  const first = await screen.findByRole("option", { name: /Calibration/ });
  const second = await screen.findByRole("option", { name: /Fasteners/ });
  await fireEvent.click(first);
  first.focus();
  await fireEvent.keyDown(first, { key: "F2" });
  const name = (await screen.findByLabelText("Name")) as HTMLInputElement;
  await waitFor(() => expect(document.activeElement).toBe(name));
  first.focus();
  await fireEvent.keyDown(first, { key: "Escape" });
  await fireEvent.click(second);
  await screen.findByText("Group 'Fasteners'");
  second.focus();
  await waitFor(() => expect(document.activeElement).toBe(second));
  expect(screen.queryByLabelText("Name")).toBeNull();
});

it.each(["click", "contextmenu"])(
  "cancels pending %s scrolling when the grid is destroyed",
  async (eventName) => {
    const { models, stream } = setup();
    render(ModelGrid, { modelStream: stream(models) as IModelStreamManager });
    const option = await screen.findByRole("option", { name: /Part 1/ });
    const scroll = vi.spyOn(option, "scrollIntoView");
    vi.useFakeTimers();
    try {
      option.dispatchEvent(new MouseEvent(eventName, { bubbles: true }));
      cleanup();
      vi.advanceTimersByTime(30);
      expect(scroll).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  },
);
