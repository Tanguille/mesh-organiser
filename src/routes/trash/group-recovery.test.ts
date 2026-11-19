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
import { IGroupApi } from "#lib/api/shared/group_api";
import {
  createModelInstance,
  IModelApi,
  type Model,
} from "#lib/api/shared/model_api";
import { configurationMeta } from "#lib/configuration.svelte";
import Page from "./+page.svelte";

vi.mock("svelte-sonner", () => ({
  toast: { promise: vi.fn(), error: vi.fn(), success: vi.fn() },
}));

interface DeletedGroup {
  id: number;
  name: string;
  deleted: Date;
  groupGlobalId: string;
  deletionOperationId: string | null;
}

const deletedAt = new Date(Date.now() - 24 * 60 * 60 * 1000);
const group = (): DeletedGroup => ({
  id: 1,
  name: "Cabinet",
  deleted: deletedAt,
  groupGlobalId: "group-1",
  deletionOperationId: "deletion-1",
});
function model(): Model {
  return createModelInstance(
    1,
    "Benchy",
    createBlobInstance(1, "sha", "stl", 1, "2024-01-01T00:00:00Z"),
    null,
    null,
    "2024-01-01T00:00:00Z",
    "2024-01-01T00:00:00Z",
    null,
    [],
    [],
    "model-1",
    deletedAt.toISOString(),
  );
}

function provideApis(
  initialGroups: DeletedGroup[],
  initialModels: Model[] = [],
) {
  let groups = initialGroups;
  let models = initialModels;
  const groupApi = {
    getTrashedGroups: vi.fn(async () => groups),
    restoreGroups: vi.fn(async (restored: DeletedGroup[]) => {
      const ids = new Set(restored.map((item) => item.id));
      groups = groups.filter((item) => !ids.has(item.id));
    }),
  };
  const modelApi = {
    getTrashedModels: vi.fn(async () => models),
    restoreModels: vi.fn(async (restored: Model[]) => {
      const ids = new Set(restored.map((item) => item.id));
      models = models.filter((item) => !ids.has(item.id));
    }),
  };
  const container = getContainer();
  container.addSingleton(IGroupApi, groupApi);
  container.addSingleton(IModelApi, modelApi);
  container.addSingleton(IBlobApi, {
    getBlobThumbnailUrl: vi.fn().mockResolvedValue(""),
  });
  return { groupApi, modelApi };
}

afterEach(() => {
  cleanup();
  resetContainer();
  configurationMeta.applicationReadOnly = false;
  vi.restoreAllMocks();
});

it("lists a deleted group even when no models are in the trash", async () => {
  provideApis([group()]);
  render(Page);
  expect(await screen.findByText("Cabinet")).toBeTruthy();
  expect(screen.getByText("Deleted 1 day ago")).toBeTruthy();
  expect(
    screen.getByRole("checkbox", { name: "Select group Cabinet" }),
  ).toBeTruthy();
  expect(screen.queryByText(/The trash is empty/)).toBeNull();
});

it("keeps the loading state until both models and groups have loaded", async () => {
  const { groupApi } = provideApis([]);
  let finishGroups!: (groups: DeletedGroup[]) => void;
  groupApi.getTrashedGroups.mockImplementationOnce(
    () => new Promise((resolve) => (finishGroups = resolve)),
  );
  render(Page);
  await waitFor(() => expect(groupApi.getTrashedGroups).toHaveBeenCalledOnce());
  expect(
    screen.getByRole("status", { name: "Loading the trash" }),
  ).toBeTruthy();
  expect(screen.queryByText(/The trash is empty/)).toBeNull();
  finishGroups([group()]);
  expect(await screen.findByText("Cabinet")).toBeTruthy();
});

it("restores only the selected group when a model has the same id", async () => {
  const { groupApi, modelApi } = provideApis([group()], [model()]);
  render(Page);
  await screen.findByText("Cabinet");
  expect(screen.getByText("Benchy")).toBeTruthy();
  await fireEvent.click(
    screen.getByRole("checkbox", { name: "Select group Cabinet" }),
  );
  await fireEvent.click(screen.getByRole("button", { name: "Restore" }));
  await waitFor(() => expect(screen.queryByText("Cabinet")).toBeNull());
  expect(screen.getByText("Benchy")).toBeTruthy();
  expect(groupApi.restoreGroups).toHaveBeenCalledExactlyOnceWith([
    expect.objectContaining({
      id: 1,
      groupGlobalId: "group-1",
      deletionOperationId: "deletion-1",
    }),
  ]);
  expect(modelApi.restoreModels).not.toHaveBeenCalled();
});

it("restores selected groups and models separately even when their ids collide", async () => {
  const { groupApi, modelApi } = provideApis([group()], [model()]);
  render(Page);
  await screen.findByText("Cabinet");
  await fireEvent.click(
    screen.getByRole("checkbox", { name: "Select group Cabinet" }),
  );
  await fireEvent.click(
    screen.getByRole("checkbox", { name: "Select Benchy" }),
  );
  await fireEvent.click(screen.getByRole("button", { name: "Restore" }));
  await waitFor(() => expect(screen.queryByText("Cabinet")).toBeNull());
  expect(modelApi.restoreModels).toHaveBeenCalledExactlyOnceWith([
    expect.objectContaining({ id: 1 }),
  ]);
  expect(groupApi.restoreGroups).toHaveBeenCalledExactlyOnceWith([
    expect.objectContaining({ id: 1 }),
  ]);
});

it("keeps a group available for retry when restoration fails", async () => {
  const { groupApi } = provideApis([group()]);
  groupApi.restoreGroups.mockRejectedValueOnce(new Error("offline"));
  render(Page);
  await screen.findByText("Cabinet");
  await fireEvent.click(
    screen.getByRole("checkbox", { name: "Select group Cabinet" }),
  );
  await fireEvent.click(screen.getByRole("button", { name: "Restore" }));
  await waitFor(() =>
    expect(groupApi.getTrashedGroups).toHaveBeenCalledTimes(2),
  );
  expect(screen.getByText("Cabinet")).toBeTruthy();
  await fireEvent.click(
    screen.getByRole("checkbox", { name: "Select group Cabinet" }),
  );
  await fireEvent.click(screen.getByRole("button", { name: "Restore" }));
  await waitFor(() => expect(screen.queryByText("Cabinet")).toBeNull());
  expect(groupApi.restoreGroups).toHaveBeenCalledTimes(2);
});

it("keeps the failed group and removes the model when only its restoration succeeds", async () => {
  const { groupApi, modelApi } = provideApis([group()], [model()]);
  groupApi.restoreGroups.mockRejectedValueOnce(new Error("offline"));
  render(Page);
  await screen.findByText("Cabinet");
  await fireEvent.click(
    screen.getByRole("checkbox", { name: "Select group Cabinet" }),
  );
  await fireEvent.click(
    screen.getByRole("checkbox", { name: "Select Benchy" }),
  );
  await fireEvent.click(screen.getByRole("button", { name: "Restore" }));
  await waitFor(() =>
    expect(groupApi.getTrashedGroups).toHaveBeenCalledTimes(2),
  );
  expect(screen.getByText("Cabinet")).toBeTruthy();
  expect(screen.queryByText("Benchy")).toBeNull();
  expect(groupApi.restoreGroups).toHaveBeenCalledOnce();
  expect(modelApi.restoreModels).toHaveBeenCalledOnce();
});

it("does not expose group restoration in a read-only view", async () => {
  const { groupApi } = provideApis([group()]);
  configurationMeta.applicationReadOnly = true;
  render(Page);
  await screen.findByText("Cabinet");
  expect(screen.queryByRole("button", { name: "Restore" })).toBeNull();
  expect(groupApi.restoreGroups).not.toHaveBeenCalled();
});

it("does not say the trash is empty when deleted groups fail to load", async () => {
  const { groupApi } = provideApis([]);
  groupApi.getTrashedGroups.mockRejectedValueOnce(new Error("offline"));
  render(Page);
  expect(await screen.findByText("Could not load the trash.")).toBeTruthy();
  expect(screen.queryByText(/The trash is empty/)).toBeNull();
});

it("keeps deleted models visible and reports when groups fail to load", async () => {
  const { groupApi } = provideApis([], [model()]);
  groupApi.getTrashedGroups.mockRejectedValueOnce(new Error("offline"));
  render(Page);

  expect(await screen.findByText("Benchy")).toBeTruthy();
  expect(
    await screen.findByText(/Could not load deleted groups/i),
  ).toBeTruthy();
  expect(screen.queryByText(/The trash is empty/)).toBeNull();
});

it("keeps deleted groups visible and reports when models fail to load", async () => {
  const { modelApi } = provideApis([group()]);
  modelApi.getTrashedModels.mockRejectedValueOnce(new Error("offline"));
  render(Page);

  expect(await screen.findByText("Cabinet")).toBeTruthy();
  expect(
    await screen.findByText(/Could not load deleted models/i),
  ).toBeTruthy();
  expect(screen.queryByText(/The trash is empty/)).toBeNull();
});

it("shows legacy groups that cannot be restored but excludes them from selection", async () => {
  const legacy = {
    ...group(),
    id: 2,
    name: "Legacy cabinet",
    groupGlobalId: "legacy-group",
    deletionOperationId: null,
  };
  const { groupApi, modelApi } = provideApis([group(), legacy], [model()]);
  render(Page);
  await screen.findByText("Legacy cabinet");
  const disabled = screen.getByRole("checkbox", {
    name: "Select group Legacy cabinet",
  }) as HTMLButtonElement;
  expect(disabled.disabled).toBe(true);
  expect(screen.getByText(/cannot.*restor|cannot be restored/i)).toBeTruthy();
  await fireEvent.click(screen.getByRole("checkbox", { name: "Select all" }));
  expect(screen.getByText("2 selected")).toBeTruthy();
  expect(disabled.getAttribute("data-state")).not.toBe("checked");
  await fireEvent.click(screen.getByRole("button", { name: "Restore" }));
  await waitFor(() => expect(screen.queryByText("Cabinet")).toBeNull());
  expect(screen.getByText("Legacy cabinet")).toBeTruthy();
  expect(groupApi.restoreGroups).toHaveBeenCalledExactlyOnceWith([
    expect.objectContaining({ id: 1, deletionOperationId: "deletion-1" }),
  ]);
  expect(modelApi.restoreModels).toHaveBeenCalledOnce();
});
