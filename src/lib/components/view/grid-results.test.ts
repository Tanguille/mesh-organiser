// @vitest-environment jsdom
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/svelte";
import { afterEach, expect, it, vi } from "vitest";
import { createRawSnippet } from "svelte";
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
import RightClickModels from "./right-click-models.svelte";

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

it.each(["models", "groups"] as const)(
  "shows a loading state while %s are being fetched",
  async (kind) => {
    const { stream } = setup();
    const data = stream([]);
    data.fetch.mockReset().mockImplementation(() => new Promise(() => {}));
    if (kind === "models") {
      render(ModelGrid, { modelStream: data as IModelStreamManager });
    } else {
      render(GroupGrid, { groupStream: data as IGroupStreamManager });
    }
    await waitFor(() => expect(data.fetch).toHaveBeenCalled());
    expect(
      screen.queryByText(new RegExp(`Loading ${kind}`, "i")),
    ).not.toBeNull();
    expect(screen.queryByText(new RegExp(`No ${kind} found`, "i"))).toBeNull();
  },
);

it.each(["models", "groups"] as const)(
  "shows an actionable empty %s collection",
  async (kind) => {
    const { stream } = setup();
    const data = stream([]);
    if (kind === "models")
      render(ModelGrid, { modelStream: data as IModelStreamManager });
    else render(GroupGrid, { groupStream: data as IGroupStreamManager });
    await waitFor(() => expect(data.fetch).toHaveBeenCalled());
    await waitFor(() =>
      expect(
        screen.queryByText(new RegExp(`No ${kind} found`, "i")),
      ).not.toBeNull(),
    );
    expect(screen.getByRole("link", { name: /import/i })).toBeTruthy();
  },
);

it.each(["models", "groups"] as const)(
  "distinguishes an empty %s search from an empty library",
  async (kind) => {
    const { stream } = setup();
    const data = stream([]);
    if (kind === "models")
      render(ModelGrid, { modelStream: data as IModelStreamManager });
    else render(GroupGrid, { groupStream: data as IGroupStreamManager });
    await waitFor(() => expect(data.fetch).toHaveBeenCalled());
    await fireEvent.input(screen.getByPlaceholderText("Search"), {
      target: { value: "unmatched" },
    });
    await waitFor(() =>
      expect(data.setSearchText).toHaveBeenCalledWith("unmatched"),
    );
    await waitFor(() =>
      expect(
        screen.queryByText(new RegExp(`No matching ${kind}`, "i")),
      ).not.toBeNull(),
    );
    expect(screen.getByRole("button", { name: /clear filters/i })).toBeTruthy();
  },
);

it.each(["models", "groups"] as const)(
  "clears search, pending input, and file type in %s results",
  async (kind) => {
    const { stream } = setup();
    const data = stream([]);
    if (kind === "models")
      render(ModelGrid, { modelStream: data as IModelStreamManager });
    else render(GroupGrid, { groupStream: data as IGroupStreamManager });
    await waitFor(() => expect(data.fetch).toHaveBeenCalled());
    await fireEvent.click(screen.getByTitle("File type filter"));
    await fireEvent.pointerUp(
      await screen.findByRole("option", { name: "Stl" }),
      { button: 0 },
    );
    await waitFor(() =>
      expect(data.setFileTypes).toHaveBeenCalledWith(
        expect.arrayContaining([expect.anything()]),
      ),
    );
    const input = screen.getByPlaceholderText("Search") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "absent" } });
    await waitFor(() =>
      expect(data.setSearchText).toHaveBeenCalledWith("absent"),
    );
    await screen.findByText(new RegExp(`No matching ${kind}`, "i"));
    await fireEvent.input(input, { target: { value: "pending" } });
    await fireEvent.click(
      screen.getByRole("button", { name: /clear filters/i }),
    );
    expect(input.value).toBe("");
    await waitFor(() => expect(data.setFileTypes).toHaveBeenLastCalledWith([]));
    await new Promise((resolve) => setTimeout(resolve, 250));
    expect(data.setSearchText).not.toHaveBeenCalledWith("pending");
    expect(data.setSearchText).toHaveBeenLastCalledWith(null);
  },
);

it.each(["models", "groups"] as const)(
  "shows retry instead of empty state when %s load fails",
  async (kind) => {
    const { stream } = setup();
    const data = stream([]);
    data.fetch
      .mockReset()
      .mockRejectedValueOnce(new Error("offline"))
      .mockResolvedValue([]);
    vi.spyOn(console, "error").mockImplementation(() => {});
    if (kind === "models")
      render(ModelGrid, { modelStream: data as IModelStreamManager });
    else render(GroupGrid, { groupStream: data as IGroupStreamManager });
    const retry = await screen.findByRole("button", { name: /retry/i });
    expect(screen.queryByText(new RegExp(`No ${kind} found`, "i"))).toBeNull();
    await fireEvent.click(retry);
    await waitFor(() => expect(data.fetch).toHaveBeenCalledTimes(2));
    await waitFor(() =>
      expect(
        screen.queryByText(new RegExp(`No ${kind} found`, "i")),
      ).not.toBeNull(),
    );
  },
);

it("does not offer folder or slicer actions when platform APIs are unavailable", async () => {
  const { models, stream } = setup();
  getContainer().addSingleton(ISlicerApi, undefined);
  void stream;
  render(RightClickModels, {
    models,
    children: createRawSnippet(() => ({ render: () => "<span>Models</span>" })),
  });
  await fireEvent.contextMenu(screen.getByText("Models"));
  await screen.findByRole("menuitem", { name: /selected/i });
  expect(
    screen.queryByRole("menuitem", { name: /open in folder/i }),
  ).toBeNull();
  expect(
    screen.queryByRole("menuitem", { name: /open in slicer/i }),
  ).toBeNull();
});
